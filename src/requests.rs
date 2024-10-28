use std::collections::HashMap;
use tokio::io::{ErrorKind};
use tokio::fs::*;
use tokio::net::TcpStream;
use regex::*;
use glob::*;
use crate::util::*;
use crate::pages::not_found::not_found;

pub enum Request {
    Get {resource: String, params: Option<HashMap<String, String>>, headers: HashMap<String, String>},
    Head {resource: String, headers: HashMap<String, String>},
    Post {resource: String, headers: HashMap<String, String>, data: String},
    Put {resource: String, headers: HashMap<String, String>, data: String},
    Delete {resource: String, headers: HashMap<String, String>},
    Connect {resource: String, headers: HashMap<String, String>},
    Options {resource: String, headers: HashMap<String, String>},
    Trace {resource: String, headers: HashMap<String, String>},
    Patch {resource: String, headers: HashMap<String, String>, data: String},
}

pub enum RequestData<'a> {
    Get {params: &'a Option<HashMap<String, String>>, headers: &'a HashMap<String, String>},
    Post {headers: &'a HashMap<String, String>, data: &'a String},
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
                params.insert(kv[..kv.find('=').unwrap()].to_string(), kv[kv.find('=').unwrap() + 1..].to_string());
            }
        }

        let regex_headers = Regex::new(r#"^([[:alnum:]]+(([-_])[[:alnum:]]+)*)(: )([A-Za-z0-9_ :;.,/"'?!(){}\[\]@<>=\-+*#$&`|~^%]+)$"#).unwrap();

        let headers_iter = request_string.lines().skip(1)
            .take_while(|x| {
                regex_headers.is_match(x)
            });

        let mut headers: HashMap<String, String> = HashMap::new();

        for header in headers_iter {
            headers.insert(header[..header.find(':').unwrap()].to_string(), header[header.find(':').unwrap() + 2..].to_string());
        }

        let data_iter = request_string.lines().skip(1)
            .skip_while(|x| {
                regex_headers.is_match(x)
            });

        let mut data = String::new();
        for line in data_iter {
            data.push_str(line);
        }

        let req = match req_type.trim() {
            "GET" => Self::Get {resource, params: if params.is_empty() {None} else {Some(params)}, headers},
            "HEAD" => Self::Head {resource, headers},
            "POST" => Self::Post {resource, headers, data: if data.is_empty() {return Err(ErrorKind::InvalidInput)} else {data}},
            "PUT" => Self::Put {resource, headers, data: if data.is_empty() {return Err(ErrorKind::InvalidInput)} else {data}},
            "DELETE" => Self::Delete {resource, headers},
            "CONNECT" => Self::Connect {resource, headers},
            "OPTIONS" => Self::Options {resource, headers},
            "TRACE" => Self::Trace {resource, headers},
            "PATCH" => Self::Patch {resource, headers, data: if data.is_empty() {return Err(ErrorKind::InvalidInput)} else {data}},
            _ => return Err(ErrorKind::InvalidInput)
        };
        Ok(req)
    }
}

pub async fn handle_get(mut stream: TcpStream, headers: &HashMap<String, String>, resource: &String, _parameters: &Option<HashMap<String, String>>) -> Result<(), ErrorKind> {
    let mut resource_clone = resource.clone();
    resource_clone.remove(0);

    for (k, v) in config(&mut stream).await.access_control {
        if let Ok(paths) = glob(&*k) {
            for entry in paths.filter_map(Result::ok) {
                if entry.to_string_lossy().eq(&resource_clone) {
                    if v.eq("deny") {
                        not_found(&mut stream, headers).await?;
                        return Ok(());
                    }
                }
            }
        }
    }

    if resource_clone.is_empty() {
        resource_clone = String::from("index");
    }

    match resource_clone.as_str() {
        _ => ()
    }

    let file = File::open(&resource_clone).await;
    let mut content = String::new();

    match file {
        Ok(mut f) => {
            rts_wrapper(&mut f, &mut content, &mut stream).await;

            let mut response_headers: HashMap<String, String> = HashMap::new();

            let type_guess = if let Some(guess) = mime_guess::from_path(resource_clone).first() {
                guess.to_string()
            } else {
                String::from("application/octet-stream")
            };

            response_headers.insert(String::from("Content-Type"), type_guess);

            if let Some(encodings_str) = headers.get("Accept-Encoding") {
                let encodings: Vec<String> = encodings_str.split(',').map(|x| String::from(x.trim())).collect();

                if encodings.contains(&String::from("gzip")) {
                    response_headers.insert(String::from("Content-Encoding"), String::from("gzip"));
                }
                response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
            }

            send_response(&mut stream, 200, Some(response_headers), Some(content), false).await
        },
        Err(_) => {
            not_found(&mut stream, headers).await
        }
    }
}

pub async fn handle_head(mut stream: TcpStream, headers: &HashMap<String, String>, resource: &String,) -> Result<(), ErrorKind> {
    let mut resource_clone = resource.clone();
    resource_clone.remove(0);

    for (k, v) in config(&mut stream).await.access_control {
        if let Ok(paths) = glob(&*k) {
            for entry in paths.filter_map(Result::ok) {
                if entry.to_string_lossy().eq(&resource_clone) {
                    if v.eq("deny") {
                        not_found(&mut stream, headers).await?;
                        return Ok(());
                    }
                }
            }
        }
    }

    if resource_clone.is_empty() {
        resource_clone = String::from("index");
    }

    match resource_clone.as_str() {
        _ => {}
    }

    let file = File::open(resource_clone).await;
    let mut content = String::new();

    match file {
        Ok(mut f) => {
            rts_wrapper(&mut f, &mut content, &mut stream).await;

            let content_length_string = content.len().to_string();
            let content_length_header = HashMap::from([(String::from("Content-Length"), content_length_string)]);

            send_response(&mut stream, 200, Some(content_length_header), None, false).await
        },
        Err(_) => {
            not_found(&mut stream, headers).await
        }
    }
}

pub async fn handle_post(mut stream: TcpStream, headers: &HashMap<String, String>, resource: &String, _data: &String) -> Result<(), ErrorKind> {
    let mut resource_clone = resource.clone();
    resource_clone.remove(0);

    for (k, v) in config(&mut stream).await.access_control {
        if let Ok(paths) = glob(&*k) {
            for entry in paths.filter_map(Result::ok) {
                if entry.to_string_lossy().eq(&resource_clone) {
                    if v.eq("deny") {
                        not_found(&mut stream, headers).await?;
                        return Ok(());
                    }
                }
            }
        }
    }

    if !headers.get("Content-Type").unwrap_or(&String::from("application/x-www-form-urlencoded")).eq("application/x-www-form-urlencoded") {
        let accept_mime_header = HashMap::from([(String::from("Accept"), String::from("application/x-www-form-urlencoded"))]);

        return send_response(&mut stream, 415, Some(accept_mime_header), None, false).await;
    }

    if resource_clone.is_empty() {
        resource_clone = String::from("index");
    }

    match resource_clone.as_str() {
        _ => ()
    }

    let file = File::open(resource_clone).await;

    let mut content = String::new();

    match file {
        Ok(mut f) => {
            rts_wrapper(&mut f, &mut content, &mut stream).await;
            send_response(&mut stream, 204, None, Some(content), false).await
        },
        Err(_) => {
            not_found(&mut stream, headers).await
        }
    }
}

pub async fn handle_options(mut stream: TcpStream, _headers: &HashMap<String, String>, _resource: &String) -> Result<(), ErrorKind> {
    let accept_header = HashMap::from([(String::from("Accept"), String::from("GET, HEAD, POST, OPTIONS"))]);

    send_response(&mut stream, 204, Some(accept_header), None, false).await
}