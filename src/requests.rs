use std::error::Error;
use std::collections::HashMap;
use std::str::FromStr;
use tokio::fs::*;
use regex::*;
use libloading::Error as LibError;
use mime_guess::Mime;
use tokio::io::{AsyncRead, AsyncWrite};
use crate::util::*;
use crate::config::Config;
use drain_common::RequestData::{*};
use crate::error::ServerError;

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

impl Request {
    pub fn parse_from_string(request_string: &String) -> Result<Self, ServerError> {
        let general_regex = Regex::new(
            r#"^((GET|HEAD|POST|PUT|DELETE|CONNECT|OPTIONS|TRACE|PATCH) /(((([A-Za-z0-9\-_]+\.[[:alnum:]]+/?)+)+|([A-Za-z0-9\-_]+/?)+)+(\?([[:alnum:]]+=[[:alnum:]]+)(&[[:alnum:]]+=[[:alnum:]]+)*)?)? (HTTP/((0\.9)|(1\.0)|(1\.1)|(2)|(3))))(\r\n(([[:alnum]]+(([-_])[[:alnum:]]+)*)(: )([A-Za-z0-9_ :;.,/"'?!(){}\[\]@<>=\-+*#$&`|~^%]+)))*[\S\s]*\z"#
        ).unwrap();

        if !general_regex.is_match(request_string.as_str()) {
            return Err(ServerError::InvalidRequest);
        }

        let mut request_iter = request_string.lines();

        let request_line = &request_iter.next().unwrap();
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
                    return Err(ServerError::InvalidRequest);
                }
            }
        }

        let regex_headers = Regex::new(r#"^([[:alnum:]]+(([-_])[[:alnum:]]+)*)(: )([A-Za-z0-9_ :;.,/"'?!(){}\[\]@<>=\-+*#$&`|~^%]+)$"#).unwrap();

        let headers_iter = request_iter
            .take_while(|x| {
                regex_headers.is_match(x)
            });

        let mut headers: HashMap<String, String> = HashMap::new();

        for header in headers_iter {
            headers.insert(header[..header.find(':').unwrap()].to_lowercase(), header[header.find(':').unwrap() + 2..].to_string());
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
            _ => return Err(ServerError::InvalidRequest),
        };
        Ok(req)
    }
}

pub async fn handle_get<T>(mut stream: T, config: &Config, headers: &HashMap<String, String>, mut resource: String, params: &Option<HashMap<String, String>>) -> Result<(), Box<dyn Error>>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    let document_root = &config.document_root;
    resource.remove(0);

    let mut response_headers: HashMap<String, String> = HashMap::new();

    if resource.is_empty() {
        resource = if let Ok(_) = File::open(format!("{document_root}/index.html")).await {
            format!("{document_root}/index.html")
        } else {
            String::from("index")
        };
    }

    if !config.is_access_allowed(&resource, &mut stream).await {
        let deny_action = config.get_deny_action();
        let content = page(if deny_action == 404 {"not_found"} else {"forbidden"}, Get {params: &None, headers}, &mut response_headers, config).await;
        let content_type = response_headers.get("Content-Type");

        if let (Ok(Some(c)), Some(c_t)) = (content, content_type) {
            let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                (mime.to_string(), mime.type_().to_string())
            } else {
                response_headers.remove(&String::from("Content-Type"));
                return send_response(&mut stream, Some(config), deny_action, Some(response_headers), None).await
            };

            if let Some(encoding) = config.get_response_encoding(&c, &mime_type, &general_type, headers) {
                response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
            }

            return send_response(&mut stream, Some(config), deny_action, Some(response_headers), Some(c)).await
        }
        return send_response(&mut stream, Some(config), deny_action, Some(response_headers), None).await
    }

    if config.dynamic_pages.contains(&resource) {
        let content = page(&*resource, Get {params, headers}, &mut response_headers, config).await;
        let content_type = response_headers.get("Content-Type");

        match (content, content_type) {
            (Ok(Some(c)), Some(c_t)) => {
                let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                    (mime.to_string(), mime.type_().to_string())
                } else {
                    response_headers.remove(&String::from("Content-Type"));
                    return send_response(&mut stream, Some(config), 200, Some(response_headers), None).await
                };

                if let Some(encoding) = config.get_response_encoding(&c, &mime_type, &general_type, headers) {
                    response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                    response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
                }

                if response_headers.contains_key("Location") || response_headers.contains_key("location") {
                    return send_response(&mut stream, Some(config), 302, Some(response_headers), Some(c)).await;
                }

                return send_response(&mut stream, Some(config), 200, Some(response_headers), Some(c)).await;
            },
            (Ok(None), _) | (Ok(Some(_)), None) => {
                if response_headers.contains_key("Location") || response_headers.contains_key("location") {
                    return send_response(&mut stream, Some(config), 302, Some(response_headers), None).await;
                }

                return send_response(&mut stream, Some(config), 200, Some(response_headers), None).await;
            },
            (Err(e), _) => {
                match e {
                    LibError::DlSym {..} => {},
                    _ => {
                        eprintln!("[handle_post():{}] An error occurred while opening a dynamic library file. Check if dynamic_pages_library field in config.json is correct.", line!());
                        return Err(Box::new(e));
                    }
                }
            }
        }
    }

    let file = File::open(format!("{document_root}/{}", &resource)).await;

    match file {
        Ok(mut f) => {
            let mut content: Vec<u8> = Vec::new();
            rte_wrapper(&mut f, &mut content, &mut stream).await;
            let content_empty = content.is_empty();

            let (guess, general_type) = if let Some(guess) = mime_guess::from_path(resource).first() {
                (guess.to_string(), guess.type_().to_string())
            } else {
                (String::from("application/octet-stream"), String::from("application"))
            };

            if let Some(encoding) = config.get_response_encoding(&content, &guess, &general_type, headers) {
                response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
            }

            response_headers.insert(String::from("Content-Type"), guess);

            send_response(&mut stream, Some(config), 200, Some(response_headers), if !content_empty {Some(content)} else {None}).await
        },
        Err(_) => {
            let content = page("not_found", Get {params: &None, headers}, &mut response_headers, config).await;
            let content_type = response_headers.get("Content-Type");

            if let (Ok(Some(c)), Some(c_t)) = (content, content_type) {
                let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                    (mime.to_string(), mime.type_().to_string())
                } else {
                    response_headers.remove(&String::from("Content-Type"));
                    return send_response(&mut stream, Some(config), 404, Some(response_headers), None).await
                };

                if let Some(encoding) = config.get_response_encoding(&c, &mime_type, &general_type, headers) {
                    response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                    response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
                }

                return send_response(&mut stream, Some(config), 404, Some(response_headers), Some(c)).await;
            }
            send_response(&mut stream, Some(config), 404, Some(response_headers), None).await
        }
    }
}

pub async fn handle_head<T>(mut stream: T, config: &Config, headers: &HashMap<String, String>, mut resource: String) -> Result<(), Box<dyn Error>>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    let document_root = &config.document_root;
    resource.remove(0);

    let mut response_headers: HashMap<String, String> = HashMap::new();

    if resource.is_empty() {
        resource = if let Ok(_) = File::open(format!("{document_root}/index.html")).await {
            format!("{document_root}/index.html")
        } else {
            String::from("index")
        };
    }

    if !config.is_access_allowed(&resource, &mut stream).await {
        let deny_action = config.get_deny_action();
        return send_response(&mut stream, Some(config), deny_action, Some(response_headers), None).await;
    }

    if config.dynamic_pages.contains(&resource) {
        match page(&*resource, Head {headers}, &mut response_headers, config).await {
            Ok(content) => {
                if let Some(c) = content {
                    let content_length = c.len().to_string();
                    response_headers.insert(String::from("Content-Length"), content_length);
                }

                if response_headers.contains_key("Location") || response_headers.contains_key("location") {
                    return send_response(&mut stream, Some(config), 302, Some(response_headers), None).await;
                }

                return send_response(&mut stream, Some(config), 200, Some(response_headers), None).await;
            },
            Err(e) => {
                match e {
                    LibError::DlSym {..} => {},
                    _ => {
                        eprintln!("[handle_head():{}] An error occurred while opening a dynamic library file. Check if dynamic_pages_library field in config.json is correct.", line!());
                        return Err(Box::new(e));
                    }
                }
            }
        }
    }

    let file = File::open(format!("{document_root}/{}", &resource)).await;

    match file {
        Ok(mut f) => {
            let mut content: Vec<u8> = Vec::new();
            rte_wrapper(&mut f, &mut content, &mut stream).await;

            let content_length = content.len().to_string();
            response_headers.insert(String::from("Content-Length"), content_length);

            send_response(&mut stream, Some(config), 200, Some(response_headers), None).await
        },
        Err(_) => {
            send_response(&mut stream, Some(config), 404, Some(response_headers), None).await
        }
    }
}

pub async fn handle_post<T>(mut stream: T, config: &Config, headers: &HashMap<String, String>, mut resource: String, data: &Option<HashMap<String, String>>) -> Result<(), Box<dyn Error>>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    let document_root = &config.document_root;
    resource.remove(0);

    let mut response_headers: HashMap<String, String> = HashMap::new();

    if resource.is_empty() {
        resource = if let Ok(_) = File::open(format!("{document_root}/index.html")).await {
            format!("{document_root}/index.html")
        } else {
            String::from("index")
        };
    }

    if !config.is_access_allowed(&resource, &mut stream).await {
        let deny_action = config.get_deny_action();
        let content = page(if deny_action == 404 {"not_found"} else {"forbidden"}, Post {data: &None, headers}, &mut response_headers, config).await;
        let content_type = response_headers.get("Content-Type");

        if let (Ok(Some(c)), Some(c_t)) = (content, content_type) {
            let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                (mime.to_string(), mime.type_().to_string())
            } else {
                response_headers.remove(&String::from("Content-Type"));
                return send_response(&mut stream, Some(config), deny_action, Some(response_headers), None).await
            };

            if let Some(encoding) = config.get_response_encoding(&c, &mime_type, &general_type, headers) {
                response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
            }

            return send_response(&mut stream, Some(config), deny_action, Some(response_headers), Some(c)).await
        }
        return send_response(&mut stream, Some(config), deny_action, Some(response_headers), None).await
    }

    if !headers.get("Content-Type").unwrap_or(&String::from("application/x-www-form-urlencoded")).eq("application/x-www-form-urlencoded") {
        response_headers.insert(String::from("Accept-Post"), String::from("application/x-www-form-urlencoded"));
        response_headers.insert(String::from("Vary"), String::from("Content-Type"));

        return send_response(&mut stream, Some(config), 415, Some(response_headers), None).await;
    }

    if config.dynamic_pages.contains(&resource) {
        let content = page(&*resource, Post {data, headers}, &mut response_headers, config).await;
        let content_type = response_headers.get("Content-Type");

        match (content, content_type) {
            (Ok(Some(c)), Some(c_t)) => {
                let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                    (mime.to_string(), mime.type_().to_string())
                } else {
                    response_headers.remove(&String::from("Content-Type"));
                    return send_response(&mut stream, Some(config), 200, Some(response_headers), None).await
                };

                if let Some(encoding) = config.get_response_encoding(&c, &mime_type, &general_type, headers) {
                    response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                    response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
                }

                if response_headers.contains_key("Location") || response_headers.contains_key("location") {
                    return send_response(&mut stream, Some(config), 302, Some(response_headers), Some(c)).await;
                }

                return send_response(&mut stream, Some(config), 200, Some(response_headers), Some(c)).await;
            },
            (Ok(None), _) | (Ok(Some(_)), None) => {
                if response_headers.contains_key("Location") || response_headers.contains_key("location") {
                    return send_response(&mut stream, Some(config), 302, Some(response_headers), None).await;
                }

                return send_response(&mut stream, Some(config), 200, Some(response_headers), None).await;
            },
            (Err(e), _) => {
                match e {
                    LibError::DlSym {..} => {},
                    _ => {
                        eprintln!("[handle_post():{}] An error occurred while opening a dynamic library file. Check if dynamic_pages_library field in config.json is correct.", line!());
                        return Err(Box::new(e));
                    }
                }
            }
        }
    }

    let file = File::open(format!("{document_root}/{}", &resource)).await;

    match file {
        Ok(mut f) => {
            let mut content: Vec<u8> = Vec::new();
            rte_wrapper(&mut f, &mut content, &mut stream).await;
            let content_empty = content.is_empty();

            let (guess, general_type) = if let Some(guess) = mime_guess::from_path(resource).first() {
                (guess.to_string(), guess.type_().to_string())
            } else {
                (String::from("application/octet-stream"), String::from("application"))
            };

            if let Some(encoding) = config.get_response_encoding(&content, &guess, &general_type, headers) {
                response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
            }

            response_headers.insert(String::from("Content-Type"), guess);

            send_response(&mut stream, Some(config), 204, Some(response_headers), if !content_empty {Some(content)} else {None}).await
        },
        Err(_) => {
            let content = page("not_found", Post {data: &data, headers}, &mut response_headers, config).await;
            let content_type = response_headers.get("Content-Type");

            if let (Ok(Some(c)), Some(c_t)) = (content, content_type) {
                let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                    (mime.to_string(), mime.type_().to_string())
                } else {
                    response_headers.remove(&String::from("Content-Type"));
                    return send_response(&mut stream, Some(config), 404, Some(response_headers), None).await
                };

                if let Some(encoding) = config.get_response_encoding(&c, &mime_type, &general_type, headers) {
                    response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                    response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
                }

                return send_response(&mut stream, Some(config), 404, Some(response_headers), Some(c)).await;
            }
            send_response(&mut stream, Some(config), 404, Some(response_headers), None).await
        }
    }
}

pub async fn handle_options<T>(mut stream: T, config: &Config) -> Result<(), Box<dyn Error>>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    let response_headers = HashMap::from([(String::from("Accept"), String::from("GET, HEAD, POST, OPTIONS"))]);

    send_response(&mut stream, Some(config),204, Some(response_headers), None).await
}