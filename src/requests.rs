use std::error::Error;
use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;
use bstr::ByteSlice;
use tokio::fs::*;
use regex::*;
use libloading::Error as LibError;
use mime_guess::Mime;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use crate::util::*;
use crate::config::CONFIG;
use crate::error::ServerError;
use crate::pages::index_of::index_of;
use crate::pages::internal_server_error::internal_server_error;
use drain_common::RequestBody;
use drain_common::RequestData::*;
use drain_common::cookies::SetCookie;

pub enum Request {
    Get {resource: String, params: Option<HashMap<String, String>>, headers: HashMap<String, String>},
    Head {resource: String, headers: HashMap<String, String>},
    Post {resource: String, headers: HashMap<String, String>, data: Option<RequestBody>},
    Put {resource: String, headers: HashMap<String, String>, data: Option<RequestBody>},
    Delete {resource: String, headers: HashMap<String, String>},
    Connect {resource: String, headers: HashMap<String, String>},
    Options {resource: String, headers: HashMap<String, String>},
    Trace {resource: String, headers: HashMap<String, String>},
    Patch {resource: String, headers: HashMap<String, String>, data: Option<RequestBody>},
}

impl Request {
    pub fn parse_from_string(request_string: &String) -> Result<Self, ServerError> {
        let general_regex = Regex::new(
            r#"^((GET|HEAD|POST|PUT|DELETE|CONNECT|OPTIONS|TRACE|PATCH) /(((([A-Za-z0-9\-_]*\.[[:alnum:]]+/?)+)+|([A-Za-z0-9\-_]+/?)+)+(\?([[:alnum:]]+=[[:alnum:]]+)(&[[:alnum:]]+=[[:alnum:]]+)*)?)? (HTTP/((0\.9)|(1\.0)|(1\.1)|(2)|(3))))(\r\n(([[:alnum]]+(([-_])[[:alnum:]]+)*)(: )([A-Za-z0-9_ :;.,/"'?!(){}\[\]@<>=\-+*#$&`|~^%]+)))*[\S\s]*\z"#
        ).unwrap();

        if !general_regex.is_match(request_string.as_str()) {
            return Err(ServerError::InvalidRequest);
        }

        let mut request_iter = request_string.lines();

        let request_line = &request_iter.next().unwrap();
        let mut iter_req_line = request_line.split_whitespace();

        let req_type = iter_req_line.next().unwrap().to_uppercase();
        let resource_with_params = iter_req_line.next().unwrap().trim();
        let mut resource = String::from(resource_with_params);
        let mut params: HashMap<String, String> = HashMap::new();

        if request_line.contains('?') {
            let resource_split = resource_with_params.split_once('?').unwrap();
            resource = String::from(resource_split.0);
            let params_str = String::from(resource_split.1);

            for kv in params_str.split('&') {
                let param_split = kv.split_once('=').unwrap();
                if let Some(_) = params.insert(String::from(param_split.0), String::from(param_split.1)) {
                    return Err(ServerError::InvalidRequest);
                }
            }
        }

        let headers_iter = request_iter
            .take_while(|x| {
                HEADERS_REGEX.is_match(x.as_bytes())
            });

        let mut headers: HashMap<String, String> = HashMap::new();

        for header in headers_iter {
            let header_split = header.split_once(':').unwrap();
            headers.insert(header_split.0.trim().to_lowercase(), String::from(header_split.1.trim()));
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

pub async fn handle_get<T>(mut stream: T, headers: &HashMap<String, String>, mut resource: String, params: &Option<HashMap<String, String>>) -> Result<(), Box<dyn Error>>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    let document_root = &CONFIG.document_root;
    resource.remove(0);

    let mut response_headers: HashMap<String, String> = HashMap::new();

    if let Some(access_control) = &CONFIG.access_control {
        if !access_control.is_access_allowed(&resource) {
            let deny_action = access_control.deny_action;
            let mut set_cookie: HashMap<String, SetCookie> = HashMap::new();
            let content = endpoint(
                if deny_action == 404 { "not_found" } else { "forbidden" },
                &mut stream,
                Get(&None),
                headers,
                &mut response_headers,
                &mut set_cookie).await;
            let content_type = response_headers.get("content-type");

            if let (Ok(Some(c)), Some(c_t)) = (content, content_type) {
                let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                    (mime.to_string(), mime.type_().to_string())
                } else {
                    response_headers.remove(&String::from("content-type"));
                    return send_response(&mut stream, deny_action, Some(response_headers), None, Some(set_cookie)).await
                };

                if let Some(encoding) = CONFIG.get_response_encoding(&c, &mime_type, &general_type, headers) {
                    response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                    response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
                }

                return send_response(&mut stream, deny_action, Some(response_headers), Some(c), Some(set_cookie)).await
            }
            return send_response(&mut stream, deny_action, Some(response_headers), None, Some(set_cookie)).await
        }
    }

    if Path::new(&format!("{document_root}/{resource}")).is_dir() {
        let res_tmp = if let Ok(_) = File::open(format!("{document_root}/{resource}/index.html")).await {
            format!("{resource}/index.html")
        } else {
            format!("{resource}/index")
        };

        if Path::new(&format!("{document_root}/{res_tmp}")).is_dir() {
            return index_of(&mut stream, resource, false, headers).await;
        }

        let res_tmp_trim = String::from(res_tmp.trim_start_matches("/"));

        if let Err(_) = File::open(format!("{document_root}/{res_tmp}")).await {
            match &CONFIG.endpoints {
                Some(endpoints) if endpoints.contains(&res_tmp_trim) => {}
                _ => {
                    return index_of(&mut stream, resource, false, headers).await;
                }
            }
        }

        resource = res_tmp_trim;
    }

    if let Some(endpoints) = &CONFIG.endpoints {
        if endpoints.contains(&resource) {
            let mut set_cookie: HashMap<String, SetCookie> = HashMap::new();
            let content = endpoint(&*resource, &mut stream, Get(params), headers, &mut response_headers, &mut set_cookie).await;
            let content_type = response_headers.get("content-type");

            match (content, content_type) {
                (Ok(Some(c)), Some(c_t)) => {
                    let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                        (mime.to_string(), mime.type_().to_string())
                    } else {
                        response_headers.remove(&String::from("content-type"));
                        return send_response(&mut stream, 200, Some(response_headers), None, Some(set_cookie)).await
                    };

                    if let Some(encoding) = CONFIG.get_response_encoding(&c, &mime_type, &general_type, headers) {
                        response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                        response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
                    }

                    if response_headers.contains_key("location") {
                        return send_response(&mut stream, 302, Some(response_headers), Some(c), Some(set_cookie)).await;
                    }

                    return send_response(&mut stream, 200, Some(response_headers), Some(c), Some(set_cookie)).await;
                },
                (Ok(None), _) | (Ok(Some(_)), None) => {
                    if response_headers.contains_key("location") {
                        return send_response(&mut stream, 302, Some(response_headers), None, Some(set_cookie)).await;
                    }

                    return send_response(&mut stream, 200, Some(response_headers), None, Some(set_cookie)).await;
                },
                (Err(e), _) => {
                    match e {
                        LibError::DlSym { .. } => {},
                        _ => {
                            eprintln!("[handle_get():{}]    An error occurred while opening a dynamic library file.\
                                                            Check if dynamic_pages_library field in config.json is correct.\n
                                                            Attempting to send Internal Server Error page to the client...", line!());
                            if let Err(e) = internal_server_error(&mut stream).await {
                                eprintln!("[handle_get():{}] FAILED. Error information:\n{e}", line!());
                            }
                            eprintln!("Attempting to close connection...");
                            if let Err(e) = stream.shutdown().await {
                                eprintln!("[handle_get():{}] FAILED. Error information:\n{e}", line!());
                            }
                            panic!("Unrecoverable error occurred while handling connection.");
                        }
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
                if content.is_utf8() {
                    (String::from("text/plain"), String::from("text"))
                } else {
                    (String::from("application/octet-stream"), String::from("application"))
                }
            };

            if let Some(encoding) = CONFIG.get_response_encoding(&content, &guess, &general_type, headers) {
                response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
            }

            response_headers.insert(String::from("Content-Type"), guess);

            send_response(&mut stream, 200, Some(response_headers), if !content_empty {Some(content)} else {None}, None).await
        },
        Err(_) => {
            let mut set_cookie: HashMap<String, SetCookie> = HashMap::new();
            let content = endpoint("not_found", &mut stream, Get(params), headers, &mut response_headers, &mut set_cookie).await;
            let content_type = response_headers.get("content-type");

            if let (Ok(Some(c)), Some(c_t)) = (content, content_type) {
                let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                    (mime.to_string(), mime.type_().to_string())
                } else {
                    response_headers.remove(&String::from("content-type"));
                    return send_response(&mut stream, 404, Some(response_headers), None, Some(set_cookie)).await
                };

                if let Some(encoding) = CONFIG.get_response_encoding(&c, &mime_type, &general_type, headers) {
                    response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                    response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
                }

                return send_response(&mut stream, 404, Some(response_headers), Some(c), Some(set_cookie)).await;
            }
            send_response(&mut stream, 404, Some(response_headers), None, Some(set_cookie)).await
        }
    }
}

pub async fn handle_head<T>(mut stream: T, headers: &HashMap<String, String>, mut resource: String) -> Result<(), Box<dyn Error>>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    let document_root = &CONFIG.document_root;
    resource.remove(0);

    let mut response_headers: HashMap<String, String> = HashMap::new();

    if let Some(access_control) = &CONFIG.access_control {
        if !access_control.is_access_allowed(&resource) {
            let deny_action = access_control.deny_action;
            return send_response(&mut stream, deny_action, Some(response_headers), None, None).await;
        }
    }

    if Path::new(&format!("{document_root}/{resource}")).is_dir() {
        let res_tmp = if let Ok(_) = File::open(format!("{document_root}/{resource}/index.html")).await {
            format!("{resource}/index.html")
        } else {
            format!("{resource}/index")
        };

        if Path::new(&format!("{document_root}/{res_tmp}")).is_dir() {
            return index_of(&mut stream, resource, true, headers).await;
        }

        let res_tmp_trim = String::from(res_tmp.trim_start_matches("/"));

        if let Err(_) = File::open(format!("{document_root}/{res_tmp}")).await {
            match &CONFIG.endpoints {
                Some(endpoints) if endpoints.contains(&res_tmp_trim) => {}
                _ => {
                    return index_of(&mut stream, resource, true, headers).await;
                }
            }
        }

        resource = res_tmp_trim;
    }

    if let Some(endpoints) = &CONFIG.endpoints {
        if endpoints.contains(&resource) {
            let mut set_cookie: HashMap<String, SetCookie> = HashMap::new();
            match endpoint(&*resource, &mut stream, Head, headers, &mut response_headers, &mut set_cookie).await {
                Ok(content) => {
                    if let Some(c) = content {
                        let content_length = c.len().to_string();
                        response_headers.insert(String::from("Content-Length"), content_length);
                    }

                    if response_headers.contains_key("location") {
                        return send_response(&mut stream, 302, Some(response_headers), None, Some(set_cookie)).await;
                    }

                    return send_response(&mut stream, 200, Some(response_headers), None, Some(set_cookie)).await;
                },
                Err(e) => {
                    match e {
                        LibError::DlSym { .. } => {},
                        _ => {
                            eprintln!("[handle_head():{}]   An error occurred while opening a dynamic library file.\
                                                            Check if dynamic_pages_library field in config.json is correct.\n
                                                            Attempting to send Internal Server Error page to the client...", line!());
                            if let Err(e) = internal_server_error(&mut stream).await {
                                eprintln!("[handle_head():{}] FAILED. Error information:\n{e}", line!());
                            }
                            eprintln!("Attempting to close connection...");
                            if let Err(e) = stream.shutdown().await {
                                eprintln!("[handle_head():{}] FAILED. Error information:\n{e}", line!());
                            }
                            panic!("Unrecoverable error occurred while handling connection.");
                        }
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

            send_response(&mut stream, 200, Some(response_headers), None, None).await
        },
        Err(_) => {
            send_response(&mut stream, 404, Some(response_headers), None, None).await
        }
    }
}

pub async fn handle_post<'a, T>(mut stream: T, headers: &HashMap<String, String>, mut resource: String, data: &Option<RequestBody>) -> Result<(), Box<dyn Error>>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    let document_root = &CONFIG.document_root;
    resource.remove(0);

    let mut response_headers: HashMap<String, String> = HashMap::new();

    if let Some(access_control) = &CONFIG.access_control {
        if !access_control.is_access_allowed(&resource) {
            let deny_action = access_control.deny_action;
            let mut set_cookie: HashMap<String, SetCookie> = HashMap::new();
            let content = endpoint(
                if deny_action == 404 { "not_found" } else { "forbidden" },
                &mut stream,
                Post(&None),
                headers,
                &mut response_headers,
                &mut set_cookie).await;
            let content_type = response_headers.get("content-type");

            if let (Ok(Some(c)), Some(c_t)) = (content, content_type) {
                let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                    (mime.to_string(), mime.type_().to_string())
                } else {
                    response_headers.remove(&String::from("content-type"));
                    return send_response(&mut stream, deny_action, Some(response_headers), None, Some(set_cookie)).await
                };

                if let Some(encoding) = CONFIG.get_response_encoding(&c, &mime_type, &general_type, headers) {
                    response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                    response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
                }

                return send_response(&mut stream, deny_action, Some(response_headers), Some(c), Some(set_cookie)).await
            }
            return send_response(&mut stream, deny_action, Some(response_headers), None, Some(set_cookie)).await
        }
    }

    if Path::new(&format!("{document_root}/{resource}")).is_dir() {
        let res_tmp = if let Ok(_) = File::open(format!("{document_root}/{resource}/index.html")).await {
            format!("{resource}/index.html")
        } else {
            format!("{resource}/index")
        };

        if Path::new(&format!("{document_root}/{res_tmp}")).is_dir() {
            return index_of(&mut stream, resource, false, headers).await;
        }

        let res_tmp_trim = String::from(res_tmp.trim_start_matches("/"));

        if let Err(_) = File::open(format!("{document_root}/{res_tmp}")).await {
            match &CONFIG.endpoints {
                Some(endpoints) if endpoints.contains(&res_tmp_trim) => {}
                _ => {
                    return index_of(&mut stream, resource, false, headers).await;
                }
            }
        }

        resource = res_tmp_trim;
    }

    if let Some(endpoints) = &CONFIG.endpoints {
        if endpoints.contains(&resource) {
            let mut set_cookie: HashMap<String, SetCookie> = HashMap::new();
            let content = endpoint(&*resource, &mut stream, Post(data), headers, &mut response_headers, &mut set_cookie).await;
            let content_type = response_headers.get("content-type");

            match (content, content_type) {
                (Ok(Some(c)), Some(c_t)) => {
                    let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                        (mime.to_string(), mime.type_().to_string())
                    } else {
                        response_headers.remove(&String::from("content-type"));
                        return send_response(&mut stream, 200, Some(response_headers), None, Some(set_cookie)).await
                    };

                    if let Some(encoding) = CONFIG.get_response_encoding(&c, &mime_type, &general_type, headers) {
                        response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                        response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
                    }

                    if response_headers.contains_key("location") {
                        return send_response(&mut stream, 302, Some(response_headers), Some(c), Some(set_cookie)).await;
                    }

                    return send_response(&mut stream, 200, Some(response_headers), Some(c), Some(set_cookie)).await;
                },
                (Ok(None), _) | (Ok(Some(_)), None) => {
                    if response_headers.contains_key("location") {
                        return send_response(&mut stream, 302, Some(response_headers), None, Some(set_cookie)).await;
                    }

                    return send_response(&mut stream, 200, Some(response_headers), None, Some(set_cookie)).await;
                },
                (Err(e), _) => {
                    match e {
                        LibError::DlSym { .. } => {},
                        _ => {
                            eprintln!("[handle_post():{}]   An error occurred while opening a dynamic library file.\
                                                            Check if dynamic_pages_library field in config.json is correct.\n
                                                            Attempting to send Internal Server Error page to the client...", line!());
                            if let Err(e) = internal_server_error(&mut stream).await {
                                eprintln!("[handle_post():{}] FAILED. Error information:\n{e}", line!());
                            }
                            eprintln!("Attempting to close connection...");
                            if let Err(e) = stream.shutdown().await {
                                eprintln!("[handle_post():{}] FAILED. Error information:\n{e}", line!());
                            }
                            panic!("Unrecoverable error occurred while handling connection.");
                        }
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
                if content.is_utf8() {
                    (String::from("text/plain"), String::from("text"))
                } else {
                    (String::from("application/octet-stream"), String::from("application"))
                }
            };

            if let Some(encoding) = CONFIG.get_response_encoding(&content, &guess, &general_type, headers) {
                response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
            }

            response_headers.insert(String::from("Content-Type"), guess);

            send_response(&mut stream, 204, Some(response_headers), if !content_empty {Some(content)} else {None}, None).await
        },
        Err(_) => {
            let mut set_cookie: HashMap<String, SetCookie> = HashMap::new();
            let content = endpoint("not_found", &mut stream, Post(data), headers, &mut response_headers, &mut set_cookie).await;
            let content_type = response_headers.get("content-type");

            if let (Ok(Some(c)), Some(c_t)) = (content, content_type) {
                let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                    (mime.to_string(), mime.type_().to_string())
                } else {
                    response_headers.remove(&String::from("content-type"));
                    return send_response(&mut stream, 404, Some(response_headers), None, Some(set_cookie)).await
                };

                if let Some(encoding) = CONFIG.get_response_encoding(&c, &mime_type, &general_type, headers) {
                    response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                    response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
                }

                return send_response(&mut stream, 404, Some(response_headers), Some(c), Some(set_cookie)).await;
            }
            send_response(&mut stream, 404, Some(response_headers), None, Some(set_cookie)).await
        }
    }
}

pub async fn handle_options<T>(mut stream: T) -> Result<(), Box<dyn Error>>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    let response_headers = HashMap::from([(String::from("Accept"), String::from("GET, HEAD, POST, OPTIONS"))]);

    send_response(&mut stream,204, Some(response_headers), None, None).await
}