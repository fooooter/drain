use std::collections::HashMap;
use tokio::io::{ErrorKind};
use tokio::fs::*;
use tokio::net::TcpStream;
use regex::*;
use glob::*;
use crate::util::*;
use crate::config::config;
use crate::requests::RequestData::{Get, Head, Post};

pub enum Request {
    Get {resource: String, params: Option<HashMap<String, String>>, headers: HashMap<String, String>},
    Head {resource: String, headers: HashMap<String, String>},
    Post {resource: String, headers: HashMap<String, String>, data: Option<HashMap<String, String>>},
    Put {resource: String, headers: HashMap<String, String>, data: Option<HashMap<String, String>>},
    Delete {resource: String, headers: HashMap<String, String>},
    Connect {resource: String, headers: HashMap<String, String>},
    Options {resource: String, headers: HashMap<String, String>},
    Trace {resource: String, headers: HashMap<String, String>},
    Patch {resource: String, headers: HashMap<String, String>, data: Option<HashMap<String, String>>},
}

pub enum RequestData<'a> {
    Get {params: &'a Option<HashMap<String, String>>, headers: &'a HashMap<String, String>},
    Post {headers: &'a HashMap<String, String>, data: &'a Option<HashMap<String, String>>},
    Head {headers: &'a HashMap<String, String>}
}

impl Request {
    pub fn parse_from_string(request_string: &String) -> Result<Self, ErrorKind> {
        let general_regex = Regex::new(
            r#"^((GET|HEAD|POST|PUT|DELETE|CONNECT|OPTIONS|TRACE|PATCH) /(((([A-Za-z0-9\-_]+\.[[:alnum:]]+/?)+)+|([A-Za-z0-9\-_]+/?)+)+(\?([[:alnum:]]+=[[:alnum:]]+)(&[[:alnum:]]+=[[:alnum:]]+)*)?)? (HTTP/((0\.9)|(1\.0)|(1\.1)|(2)|(3))))(\r\n(([[:alnum]]+(([-_])[[:alnum:]]+)*)(: )([A-Za-z0-9_ :;.,/"'?!(){}\[\]@<>=\-+*#$&`|~^%]+)))*[\S\s]*\z"#
        ).unwrap();

        if !general_regex.is_match(request_string.as_str()) {
            return Err(ErrorKind::InvalidInput);
        }

        let request_line = request_string.lines().next().unwrap();
        let mut iter_req_line = request_line.split_whitespace();

        let req_type = iter_req_line.next().unwrap().to_uppercase();
        let resource_with_params = iter_req_line.next().unwrap().trim();
        let mut resource = resource_with_params.to_string();
        let mut params: HashMap<String, String> = HashMap::new();

        if request_line.contains('?') {
            resource = resource_with_params[..resource_with_params.find('?').unwrap()].parse::<String>().unwrap();
            let params_str = resource_with_params[resource_with_params.find('?').unwrap() + 1..].parse::<String>().unwrap();

            for kv in params_str.split('&') {
                if let Some(_) = params.insert(kv[..kv.find('=').unwrap()].to_string(), kv[kv.find('=').unwrap() + 1..].to_string()) {
                    return Err(ErrorKind::InvalidInput);
                }
            }
        }

        let regex_headers = Regex::new(r#"^([[:alnum:]]+(([-_])[[:alnum:]]+)*)(: )([A-Za-z0-9_ :;.,/"'?!(){}\[\]@<>=\-+*#$&`|~^%]+)$"#).unwrap();

        let headers_iter = request_string.lines().skip(1)
            .take_while(|x| {
                regex_headers.is_match(x)
            });

        let mut headers: HashMap<String, String> = HashMap::new();

        for header in headers_iter {
            if let Some(_) = headers.insert(header[..header.find(':').unwrap()].to_string(), header[header.find(':').unwrap() + 2..].to_string()) {
                return Err(ErrorKind::InvalidInput);
            }
        }

        let req = match req_type.trim() {
            "GET" => Self::Get {resource, params: if params.is_empty() {None} else {Some(params)}, headers},
            "HEAD" => Self::Head {resource, headers},
            "POST" => Self::Post {resource, headers, data: None},
            "PUT" => Self::Put {resource, headers, data: None},
            "DELETE" => Self::Delete {resource, headers},
            "CONNECT" => Self::Connect {resource, headers},
            "OPTIONS" => Self::Options {resource, headers},
            "TRACE" => Self::Trace {resource, headers},
            "PATCH" => Self::Patch {resource, headers, data: None},
            _ => return Err(ErrorKind::InvalidInput)
        };
        Ok(req)
    }
}

pub async fn handle_get(mut stream: TcpStream, headers: &HashMap<String, String>, resource: &String, params: &Option<HashMap<String, String>>) -> Result<(), ErrorKind> {
    let mut resource_clone = resource.clone();
    resource_clone.remove(0);

    let mut response_headers: HashMap<String, String> = HashMap::new();

    if accepts_gzip(&headers) {
        response_headers.insert(String::from("Content-Encoding"), String::from("gzip"));
        response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
    }

    if resource_clone.is_empty() {
        resource_clone = if let Ok(_) = File::open("index.html").await {String::from("index.html")} else {String::from("index")};
    }

    for (k, v) in config(Some(&mut stream)).await.access_control {
        if let Ok(paths) = glob(&*k) {
            for entry in paths.filter_map(Result::ok) {
                if entry.to_string_lossy().eq(&resource_clone) {
                    if v.eq("deny") {
                        let content = page("not_found", &mut stream, Get {params: &None, headers}, &mut response_headers).await?;
                        return send_response(&mut stream, 404, Some(response_headers), Some(content), false).await;
                    }
                }
            }
        }
    }

    if config(Some(&mut stream)).await.dynamic_pages.contains(&resource_clone) {
        match page(&*resource_clone, &mut stream, Get {params, headers}, &mut response_headers).await {
            Ok(content) => return send_response(&mut stream, 200, Some(response_headers), Some(content), false).await,
            Err(e) => {
                if let ErrorKind::NotFound = e {} else {
                    return Err(e);
                }
            }
        }
    }

    let file = File::open(&resource_clone).await;
    let mut content = String::new();

    match file {
        Ok(mut f) => {
            rts_wrapper(&mut f, &mut content, &mut stream).await;

            let type_guess = if let Some(guess) = mime_guess::from_path(resource_clone).first() {
                guess.to_string()
            } else {
                String::from("application/octet-stream")
            };

            response_headers.insert(String::from("Content-Type"), type_guess);

            send_response(&mut stream, 200, Some(response_headers), Some(content), false).await
        },
        Err(_) => {
            content = page("not_found", &mut stream, Get {params: &None, headers}, &mut response_headers).await?;
            send_response(&mut stream, 404, Some(response_headers), Some(content), false).await
        }
    }
}

pub async fn handle_head(mut stream: TcpStream, headers: &HashMap<String, String>, resource: &String,) -> Result<(), ErrorKind> {
    let mut resource_clone = resource.clone();
    resource_clone.remove(0);

    let mut response_headers: HashMap<String, String> = HashMap::new();

    if resource_clone.is_empty() {
        resource_clone = if let Ok(_) = File::open("index.html").await {String::from("index.html")} else {String::from("index")};
    }

    for (k, v) in config(Some(&mut stream)).await.access_control {
        if let Ok(paths) = glob(&*k) {
            for entry in paths.filter_map(Result::ok) {
                if entry.to_string_lossy().eq(&resource_clone) {
                    if v.eq("deny") {
                        let content = page("not_found", &mut stream, Head {headers}, &mut response_headers).await?;
                        return send_response(&mut stream, 404, Some(response_headers), Some(content), false).await;
                    }
                }
            }
        }
    }

    if config(Some(&mut stream)).await.dynamic_pages.contains(&resource_clone) {
        match page(&*resource_clone, &mut stream, Head {headers}, &mut response_headers).await {
            Ok(content) => return send_response(&mut stream, 200, Some(response_headers), Some(content), false).await,
            Err(e) => {
                if let ErrorKind::NotFound = e {} else {
                    return Err(e);
                }
            }
        }
    }

    let file = File::open(resource_clone).await;
    let mut content = String::new();

    match file {
        Ok(mut f) => {
            rts_wrapper(&mut f, &mut content, &mut stream).await;

            let content_length_string = content.len().to_string();
            response_headers.insert(String::from("Content-Length"), content_length_string);

            send_response(&mut stream, 200, Some(response_headers), None, false).await
        },
        Err(_) => {
            content = page("not_found", &mut stream, Head {headers}, &mut response_headers).await?;
            send_response(&mut stream, 404, Some(response_headers), Some(content), false).await
        }
    }
}

pub async fn handle_post(mut stream: TcpStream, headers: &HashMap<String, String>, resource: &String, data: &Option<HashMap<String, String>>) -> Result<(), ErrorKind> {
    let mut resource_clone = resource.clone();
    resource_clone.remove(0);

    let mut response_headers: HashMap<String, String> = HashMap::new();

    if accepts_gzip(&headers) {
        response_headers.insert(String::from("Content-Encoding"), String::from("gzip"));
        response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
    }

    if resource_clone.is_empty() {
        resource_clone = if let Ok(_) = File::open("index.html").await {String::from("index.html")} else {String::from("index")};
    }

    for (k, v) in config(Some(&mut stream)).await.access_control {
        if let Ok(paths) = glob(&*k) {
            for entry in paths.filter_map(Result::ok) {
                if entry.to_string_lossy().eq(&resource_clone) {
                    if v.eq("deny") {
                        let content = page("not_found", &mut stream, Post {data, headers}, &mut response_headers).await?;
                        return send_response(&mut stream, 404, Some(response_headers), Some(content), false).await;
                    }
                }
            }
        }
    }

    if !headers.get("Content-Type").unwrap_or(&String::from("application/x-www-form-urlencoded")).eq("application/x-www-form-urlencoded") {
        response_headers.insert(String::from("Accept-Post"), String::from("application/x-www-form-urlencoded"));
        response_headers.insert(String::from("Vary"), String::from("Content-Type"));

        return send_response(&mut stream, 415, Some(response_headers), None, false).await;
    }

    if config(Some(&mut stream)).await.dynamic_pages.contains(&resource_clone) {
        match page(&*resource_clone, &mut stream, Post {data, headers}, &mut response_headers).await {
            Ok(content) => return send_response(&mut stream, 200, Some(response_headers), Some(content), false).await,
            Err(e) => {
                if let ErrorKind::NotFound = e {} else {
                    return Err(e);
                }
            }
        }
    }

    let file = File::open(resource_clone).await;

    let mut content = String::new();

    match file {
        Ok(mut f) => {
            rts_wrapper(&mut f, &mut content, &mut stream).await;
            send_response(&mut stream, 204, None, Some(content), false).await
        },
        Err(_) => {
            let content = page("not_found", &mut stream, Post {data: &data, headers}, &mut response_headers).await?;
            send_response(&mut stream, 404, Some(response_headers), Some(content), false).await
        }
    }
}

pub async fn handle_options(mut stream: TcpStream, _headers: &HashMap<String, String>, _resource: &String) -> Result<(), ErrorKind> {
    let response_headers = HashMap::from([(String::from("Accept"), String::from("GET, HEAD, POST, OPTIONS"))]);

    send_response(&mut stream, 204, Some(response_headers), None, false).await
}