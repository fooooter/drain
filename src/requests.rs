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
use tokio::sync::Semaphore;
use crate::util::ResourceType::{Dynamic, Static};

pub enum Request {
    Get {resource: String, params: Option<HashMap<String, String>>, headers: HashMap<String, String>},
    Head {resource: String, params: Option<HashMap<String, String>>, headers: HashMap<String, String>},
    Post {resource: String, params: Option<HashMap<String, String>>, headers: HashMap<String, String>, data: Option<RequestBody>},
    Put {resource: String, params: Option<HashMap<String, String>>, headers: HashMap<String, String>, data: Option<RequestBody>},
    Delete {resource: String, params: Option<HashMap<String, String>>, headers: HashMap<String, String>, data: Option<RequestBody>},
    Connect,
    Options,
    Trace(Vec<u8>),
    Patch {resource: String, params: Option<HashMap<String, String>>, headers: HashMap<String, String>, data: Option<RequestBody>},
}

impl Request {
    pub fn parse_from_string(request_string: &String, keep_alive: &mut bool) -> Result<Self, ServerError> {
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
        let http_version = iter_req_line.next().unwrap().trim();
        let mut resource = String::from(resource_with_params);
        let mut params: HashMap<String, String> = HashMap::new();

        if req_type.eq("TRACE") {
            return Ok(Self::Trace(Vec::from(request_string.as_bytes())));
        }

        if !http_version.eq("HTTP/1.1") {
            return Err(ServerError::VersionNotSupported);
        }

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

        if !headers.contains_key("host") {
            return Err(ServerError::InvalidRequest);
        }

        if let Some(connection) = headers.get("connection") {
            if connection.eq("close") {
                *keep_alive = false;
            }
        }

        let req = match req_type.trim() {
            "GET" => Self::Get {resource, params: if params.is_empty() {None} else {Some(params)}, headers},
            "HEAD" => Self::Head {resource, params: if params.is_empty() {None} else {Some(params)}, headers},
            "POST" => Self::Post {resource, params: if params.is_empty() {None} else {Some(params)}, headers, data: None},
            "PUT" => Self::Put {resource, params: if params.is_empty() {None} else {Some(params)}, headers, data: None},
            "DELETE" => Self::Delete {resource, params: if params.is_empty() {None} else {Some(params)}, headers, data: None},
            "CONNECT" => Self::Connect,
            "OPTIONS" => Self::Options,
            "PATCH" => Self::Patch {resource, params: if params.is_empty() {None} else {Some(params)}, headers, data: None},
            _ => return Err(ServerError::InvalidRequest),
        };
        Ok(req)
    }
}

static FILE_HANDLE_LIMIT: Semaphore = Semaphore::const_new(
    if cfg!(target_os = "linux") { 1023 }
    else if cfg!(target_os = "windows") { 16777215 }
    else if cfg!(target_os = "macos") { 10239 }
    else { 255 }
);

pub async fn handle_get<T>(stream: &mut T, headers: &HashMap<String, String>, mut resource: String, params: &Option<HashMap<String, String>>) -> Result<(), Box<dyn Error>>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    let document_root = &CONFIG.document_root;
    resource.remove(0);

    let mut response_headers: HashMap<String, String> = HashMap::new();

    if let Some(access_control) = &CONFIG.access_control {
        if !access_control.is_access_allowed(&resource) {
            let mut deny_action = access_control.deny_action;
            if let Some(library) = &*ENDPOINT_LIBRARY {
                let mut set_cookie: HashMap<String, SetCookie> = HashMap::new();
                let content = endpoint(
                    if deny_action == 404 { "not_found" } else { "forbidden" },
                    stream,
                    Get(&None),
                    headers,
                    &mut response_headers,
                    &mut set_cookie,
                    &mut deny_action,
                    library).await;
                let content_type = response_headers.get("content-type");

                if let (Ok(Some(c)), Some(c_t)) = (content, content_type) {
                    let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                        (mime.to_string(), mime.type_().to_string())
                    } else {
                        response_headers.remove(&String::from("content-type"));
                        return send_response(stream, deny_action, Some(response_headers), None, Some(set_cookie), None).await;
                    };

                    if let Some(encoding) = CONFIG.get_response_encoding(&c, &mime_type, &general_type, headers) {
                        response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                        response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
                    }

                    return send_response(stream, deny_action, Some(response_headers), Some(c), Some(set_cookie), Some(Dynamic)).await;
                }
                return send_response(stream, deny_action, Some(response_headers), None, Some(set_cookie), None).await;
            }
            return send_response(stream, deny_action, Some(response_headers), None, None, None).await;
        }
    }

    if Path::new(&format!("{document_root}/{resource}")).is_dir() {
        let _ = FILE_HANDLE_LIMIT.acquire().await?;
        let res_tmp = match File::open(format!("{document_root}/{resource}/index.html")).await {
            Ok(_) => {
                format!("{resource}/index.html")
            },
            _ => {
                format!("{resource}/index")
            }
        };

        if Path::new(&format!("{document_root}/{res_tmp}")).is_dir() {
            return index_of(stream, resource, false, headers).await;
        }

        let res_tmp_trim = String::from(res_tmp.trim_start_matches("/"));

        let _ = FILE_HANDLE_LIMIT.acquire().await?;
        if let Err(_) = File::open(format!("{document_root}/{res_tmp}")).await {
            match &CONFIG.endpoints {
                Some(endpoints) if (&ENDPOINT_LIBRARY).is_some() && endpoints.contains(&res_tmp_trim) => {}
                _ => {
                    return index_of(stream, resource, false, headers).await;
                }
            }
        }

        resource = res_tmp_trim;
    }

    if let (Some(endpoints), Some(library)) = (&CONFIG.endpoints, &*ENDPOINT_LIBRARY) {
        if endpoints.contains(&resource) {
            let mut set_cookie: HashMap<String, SetCookie> = HashMap::new();
            let mut status: u16 = 200;
            let content = endpoint(&*resource, stream, Get(params), headers, &mut response_headers, &mut set_cookie, &mut status, library).await;
            let content_type = response_headers.get("content-type");

            match (content, content_type) {
                (Ok(Some(c)), Some(c_t)) => {
                    let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                        (mime.to_string(), mime.type_().to_string())
                    } else {
                        response_headers.remove(&String::from("content-type"));
                        return send_response(stream, status, Some(response_headers), None, Some(set_cookie), None).await
                    };

                    if let Some(encoding) = CONFIG.get_response_encoding(&c, &mime_type, &general_type, headers) {
                        response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                        response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
                    }

                    if response_headers.contains_key("location") {
                        return send_response(stream, 302, Some(response_headers), Some(c), Some(set_cookie), Some(Dynamic)).await;
                    }

                    return send_response(stream, status, Some(response_headers), Some(c), Some(set_cookie), Some(Dynamic)).await;
                },
                (Ok(None), _) | (Ok(Some(_)), None) => {
                    if response_headers.contains_key("location") {
                        return send_response(stream, 302, Some(response_headers), None, Some(set_cookie), None).await;
                    }

                    return send_response(stream, status, Some(response_headers), None, Some(set_cookie), None).await;
                },
                (Err(e), _) => {
                    match e {
                        LibError::DlSym { .. } => {},
                        _ => {
                            eprintln!("[handle_get():{}] An unknown error occurred while executing the endpoint.\
                                                         Attempting to send Internal Server Error page to the client...", line!());
                            if let Err(e) = internal_server_error(stream).await {
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

    let _ = FILE_HANDLE_LIMIT.acquire().await?;
    let file = File::open(format!("{document_root}/{}", &resource)).await;

    match file {
        Ok(mut f) => {
            let mut content: Vec<u8> = Vec::new();
            rte_wrapper(&mut f, &mut content, stream).await;

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

            if content.is_empty() {
                send_response(stream, 200, Some(response_headers), None, None, None).await
            } else {
                if let Some(if_none_match) = headers.get("if-none-match") {
                    let mut excluded_etags = if_none_match.split(",")
                        .map(|e| e.trim_matches(|x: char| x.is_whitespace() || x == '"').to_string());

                    let etags = ETAGS.lock().await;
                    while let Some(etag) = excluded_etags.next() {
                        if etags.contains(&etag) {
                            response_headers.insert(String::from("ETag"), etag);
                            response_headers.insert(String::from("Cache-Control"), format!("max-age={}", CONFIG.cache_max_age));

                            return send_response(stream, 304, Some(response_headers), None, None, None).await;
                        }
                    }
                }
                send_response(stream, 200, Some(response_headers), Some(content), None, Some(Static)).await
            }
        },
        Err(_) => {
            if let Some(library) = &*ENDPOINT_LIBRARY {
                let mut set_cookie: HashMap<String, SetCookie> = HashMap::new();
                let content = endpoint("not_found", stream, Get(params), headers, &mut response_headers, &mut set_cookie, &mut 404u16, library).await;
                let content_type = response_headers.get("content-type");

                if let (Ok(Some(c)), Some(c_t)) = (content, content_type) {
                    let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                        (mime.to_string(), mime.type_().to_string())
                    } else {
                        response_headers.remove(&String::from("content-type"));
                        return send_response(stream, 404, Some(response_headers), None, Some(set_cookie), Some(Dynamic)).await;
                    };

                    if let Some(encoding) = CONFIG.get_response_encoding(&c, &mime_type, &general_type, headers) {
                        response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                        response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
                    }

                    return send_response(stream, 404, Some(response_headers), Some(c), Some(set_cookie), Some(Dynamic)).await;
                }
                return send_response(stream, 404, Some(response_headers), None, Some(set_cookie), Some(Dynamic)).await;
            }
            send_response(stream, 404, Some(response_headers), None, None, None).await
        }
    }
}

pub async fn handle_head<T>(stream: &mut T, headers: &HashMap<String, String>, mut resource: String, params: &Option<HashMap<String, String>>) -> Result<(), Box<dyn Error>>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    let document_root = &CONFIG.document_root;
    resource.remove(0);

    let mut response_headers: HashMap<String, String> = HashMap::new();

    if let Some(access_control) = &CONFIG.access_control {
        if !access_control.is_access_allowed(&resource) {
            let deny_action = access_control.deny_action;
            return send_response(stream, deny_action, Some(response_headers), None, None, None).await;
        }
    }

    if Path::new(&format!("{document_root}/{resource}")).is_dir() {
        let _ = FILE_HANDLE_LIMIT.acquire().await?;
        let res_tmp = match File::open(format!("{document_root}/{resource}/index.html")).await {
            Ok(_) => {
                format!("{resource}/index.html")
            },
            _ => {
                format!("{resource}/index")
            }
        };

        if Path::new(&format!("{document_root}/{res_tmp}")).is_dir() {
            return index_of(stream, resource, true, headers).await;
        }

        let res_tmp_trim = String::from(res_tmp.trim_start_matches("/"));

        let _ = FILE_HANDLE_LIMIT.acquire().await?;
        if let Err(_) = File::open(format!("{document_root}/{res_tmp}")).await {
            match &CONFIG.endpoints {
                Some(endpoints) if (&ENDPOINT_LIBRARY).is_some() && endpoints.contains(&res_tmp_trim) => {}
                _ => {
                    return index_of(stream, resource, true, headers).await;
                }
            }
        }

        resource = res_tmp_trim;
    }

    if let (Some(endpoints), Some(library)) = (&CONFIG.endpoints, &*ENDPOINT_LIBRARY) {
        if endpoints.contains(&resource) {
            let mut set_cookie: HashMap<String, SetCookie> = HashMap::new();
            let mut status: u16 = 200;
            match endpoint(&*resource, stream, Head(params), headers, &mut response_headers, &mut set_cookie, &mut status, library).await {
                Ok(content) => {
                    if let Some(c) = content {
                        let content_length = c.len().to_string();
                        response_headers.insert(String::from("Content-Length"), content_length);
                    }

                    if response_headers.contains_key("location") {
                        return send_response(stream, 302, Some(response_headers), None, Some(set_cookie), None).await;
                    }

                    return send_response(stream, status, Some(response_headers), None, Some(set_cookie), None).await;
                },
                Err(e) => {
                    match e {
                        LibError::DlSym { .. } => {},
                        _ => {
                            eprintln!("[handle_head():{}] An unknown error occurred while executing the endpoint.\
                                                          Attempting to send Internal Server Error page to the client...", line!());
                            if let Err(e) = internal_server_error(stream).await {
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

    let _ = FILE_HANDLE_LIMIT.acquire().await?;
    let file = File::open(format!("{document_root}/{}", &resource)).await;

    match file {
        Ok(mut f) => {
            let mut content: Vec<u8> = Vec::new();
            rte_wrapper(&mut f, &mut content, stream).await;

            if content.is_empty() {
                send_response(stream, 200, None, None, None, None).await
            } else {
                let content_length = content.len().to_string();
                response_headers.insert(String::from("Content-Length"), content_length);

                send_response(stream, 200, Some(response_headers), None, None, None).await
            }
        },
        Err(_) => {
            send_response(stream, 404, Some(response_headers), None, None, None).await
        }
    }
}

pub async fn handle_post<'a, T>(stream: &mut T, headers: &HashMap<String, String>, mut resource: String, data: &Option<RequestBody>, params: &Option<HashMap<String, String>>) -> Result<(), Box<dyn Error>>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    let document_root = &CONFIG.document_root;
    resource.remove(0);

    let mut response_headers: HashMap<String, String> = HashMap::new();

    if let Some(access_control) = &CONFIG.access_control {
        if !access_control.is_access_allowed(&resource) {
            let mut deny_action = access_control.deny_action;
            if let Some(library) = &*ENDPOINT_LIBRARY {
                let mut set_cookie: HashMap<String, SetCookie> = HashMap::new();
                let content = endpoint(
                    if deny_action == 404 { "not_found" } else { "forbidden" },
                    stream,
                    Post { data: &None, params: &None },
                    headers,
                    &mut response_headers,
                    &mut set_cookie,
                    &mut deny_action,
                    library).await;
                let content_type = response_headers.get("content-type");

                if let (Ok(Some(c)), Some(c_t)) = (content, content_type) {
                    let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                        (mime.to_string(), mime.type_().to_string())
                    } else {
                        response_headers.remove(&String::from("content-type"));
                        return send_response(stream, deny_action, Some(response_headers), None, Some(set_cookie), None).await;
                    };

                    if let Some(encoding) = CONFIG.get_response_encoding(&c, &mime_type, &general_type, headers) {
                        response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                        response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
                    }

                    return send_response(stream, deny_action, Some(response_headers), Some(c), Some(set_cookie), Some(Dynamic)).await;
                }
                return send_response(stream, deny_action, Some(response_headers), None, Some(set_cookie), None).await;
            }
            return send_response(stream, deny_action, Some(response_headers), None, None, None).await;
        }
    }

    if Path::new(&format!("{document_root}/{resource}")).is_dir() {
        let _ = FILE_HANDLE_LIMIT.acquire().await?;
        let res_tmp = match File::open(format!("{document_root}/{resource}/index.html")).await {
            Ok(_) => {
                format!("{resource}/index.html")
            },
            _ => {
                format!("{resource}/index")
            }
        };

        if Path::new(&format!("{document_root}/{res_tmp}")).is_dir() {
            return index_of(stream, resource, false, headers).await;
        }

        let res_tmp_trim = String::from(res_tmp.trim_start_matches("/"));

        let _ = FILE_HANDLE_LIMIT.acquire().await?;
        if let Err(_) = File::open(format!("{document_root}/{res_tmp}")).await {
            match &CONFIG.endpoints {
                Some(endpoints) if (&ENDPOINT_LIBRARY).is_some() && endpoints.contains(&res_tmp_trim) => {}
                _ => {
                    return index_of(stream, resource, false, headers).await;
                }
            }
        }

        resource = res_tmp_trim;
    }

    if let (Some(endpoints), Some(library)) = (&CONFIG.endpoints, &*ENDPOINT_LIBRARY) {
        if endpoints.contains(&resource) {
            let mut set_cookie: HashMap<String, SetCookie> = HashMap::new();
            let mut status: u16 = 200;
            let content = endpoint(&*resource, stream, Post {data, params}, headers, &mut response_headers, &mut set_cookie, &mut status, library).await;
            let content_type = response_headers.get("content-type");

            match (content, content_type) {
                (Ok(Some(c)), Some(c_t)) => {
                    let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                        (mime.to_string(), mime.type_().to_string())
                    } else {
                        response_headers.remove(&String::from("content-type"));
                        return send_response(stream, status, Some(response_headers), None, Some(set_cookie), None).await;
                    };

                    if let Some(encoding) = CONFIG.get_response_encoding(&c, &mime_type, &general_type, headers) {
                        response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                        response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
                    }

                    if response_headers.contains_key("location") {
                        return send_response(stream, 302, Some(response_headers), Some(c), Some(set_cookie), Some(Dynamic)).await;
                    }

                    return send_response(stream, status, Some(response_headers), Some(c), Some(set_cookie), Some(Dynamic)).await;
                },
                (Ok(None), _) | (Ok(Some(_)), None) => {
                    if response_headers.contains_key("location") {
                        return send_response(stream, 302, Some(response_headers), None, Some(set_cookie), None).await;
                    }

                    return send_response(stream, status, Some(response_headers), None, Some(set_cookie), None).await;
                },
                (Err(e), _) => {
                    match e {
                        LibError::DlSym { .. } => {},
                        _ => {
                            eprintln!("[handle_post():{}] An unknown error occurred while executing the endpoint.\
                                                          Attempting to send Internal Server Error page to the client...", line!());
                            if let Err(e) = internal_server_error(stream).await {
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

    let _ = FILE_HANDLE_LIMIT.acquire().await?;
    let file = File::open(format!("{document_root}/{}", &resource)).await;

    match file {
        Ok(mut f) => {
            let mut content: Vec<u8> = Vec::new();
            rte_wrapper(&mut f, &mut content, stream).await;
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

            if content_empty {
                send_response(stream, 200, Some(response_headers), None, None, None).await
            } else {
                if let Some(if_none_match) = headers.get("if-none-match") {
                    let mut excluded_etags = if_none_match.split(",")
                        .map(|e| e.trim_matches(|x: char| x.is_whitespace() || x == '"').to_string());

                    let etags = ETAGS.lock().await;
                    while let Some(etag) = excluded_etags.next() {
                        if etags.contains(&etag) {
                            response_headers.insert(String::from("ETag"), etag);
                            response_headers.insert(String::from("Cache-Control"), format!("max-age={}", CONFIG.cache_max_age));

                            return send_response(stream, 304, Some(response_headers), None, None, None).await;
                        }
                    }
                }
                send_response(stream, 200, Some(response_headers), Some(content), None, Some(Static)).await
            }
        },
        Err(_) => {
            if let Some(library) = &*ENDPOINT_LIBRARY {
                let mut set_cookie: HashMap<String, SetCookie> = HashMap::new();
                let content = endpoint("not_found", stream, Post { data, params }, headers, &mut response_headers, &mut set_cookie, &mut 404u16, library).await;
                let content_type = response_headers.get("content-type");

                if let (Ok(Some(c)), Some(c_t)) = (content, content_type) {
                    let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                        (mime.to_string(), mime.type_().to_string())
                    } else {
                        response_headers.remove(&String::from("content-type"));
                        return send_response(stream, 404, Some(response_headers), None, Some(set_cookie), None).await;
                    };

                    if let Some(encoding) = CONFIG.get_response_encoding(&c, &mime_type, &general_type, headers) {
                        response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                        response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
                    }

                    return send_response(stream, 404, Some(response_headers), Some(c), Some(set_cookie), Some(Dynamic)).await;
                }
                return send_response(stream, 404, Some(response_headers), None, Some(set_cookie), None).await;
            }
            send_response(stream, 404, Some(response_headers), None, None, None).await
        }
    }
}

pub async fn handle_options<T>(stream: &mut T) -> Result<(), Box<dyn Error>>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    let response_headers = HashMap::from([
        (String::from("Accept"), format!("GET, HEAD, POST, {} OPTIONS{}",
                                         if (&*ENDPOINT_LIBRARY).is_some() {"PUT, DELETE, PATCH"} else {""},
                                         if CONFIG.enable_trace {", TRACE"} else {""}))
    ]);

    send_response(stream,204, Some(response_headers), None, None, None).await
}

pub async fn handle_put<T>(stream: &mut T, headers: &HashMap<String, String>, mut resource: String, data: &Option<RequestBody>, params: &Option<HashMap<String, String>>) -> Result<(), Box<dyn Error>>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    let mut response_headers: HashMap<String, String> = HashMap::new();

    if let (Some(endpoints), Some(library)) = (&CONFIG.endpoints, &*ENDPOINT_LIBRARY) {
        resource.remove(0);

        if let Some(access_control) = &CONFIG.access_control {
            if !access_control.is_access_allowed(&resource) {
                let mut deny_action = access_control.deny_action;
                let mut set_cookie: HashMap<String, SetCookie> = HashMap::new();
                let content = endpoint(
                    if deny_action == 404 { "not_found" } else { "forbidden" },
                    stream,
                    Put { data: &None, params: &None },
                    headers,
                    &mut response_headers,
                    &mut set_cookie,
                    &mut deny_action,
                    library).await;
                let content_type = response_headers.get("content-type");

                if let (Ok(Some(c)), Some(c_t)) = (content, content_type) {
                    let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                        (mime.to_string(), mime.type_().to_string())
                    } else {
                        response_headers.remove(&String::from("content-type"));
                        return send_response(stream, deny_action, Some(response_headers), None, Some(set_cookie), None).await;
                    };

                    if let Some(encoding) = CONFIG.get_response_encoding(&c, &mime_type, &general_type, headers) {
                        response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                        response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
                    }

                    return send_response(stream, deny_action, Some(response_headers), Some(c), Some(set_cookie), Some(Dynamic)).await;
                }
                return send_response(stream, deny_action, Some(response_headers), None, Some(set_cookie), None).await;
            }
        }

        if endpoints.contains(&resource) {
            let mut set_cookie: HashMap<String, SetCookie> = HashMap::new();
            let mut status: u16 = 200;
            let content = endpoint(&*resource, stream, Put {data, params}, headers, &mut response_headers, &mut set_cookie, &mut status, library).await;
            let content_type = response_headers.get("content-type");

            match (content, content_type) {
                (Ok(Some(c)), Some(c_t)) => {
                    let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                        (mime.to_string(), mime.type_().to_string())
                    } else {
                        response_headers.remove(&String::from("content-type"));
                        return send_response(stream, status, Some(response_headers), None, Some(set_cookie), None).await;
                    };

                    if let Some(encoding) = CONFIG.get_response_encoding(&c, &mime_type, &general_type, headers) {
                        response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                        response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
                    }

                    if response_headers.contains_key("location") {
                        return send_response(stream, 302, Some(response_headers), Some(c), Some(set_cookie), Some(Dynamic)).await;
                    }

                    return send_response(stream, status, Some(response_headers), Some(c), Some(set_cookie), Some(Dynamic)).await;
                },
                (Ok(None), _) | (Ok(Some(_)), None) => {
                    if response_headers.contains_key("location") {
                        return send_response(stream, 302, Some(response_headers), None, Some(set_cookie), None).await;
                    }

                    return send_response(stream, status, Some(response_headers), None, Some(set_cookie), None).await;
                },
                (Err(e), _) => {
                    match e {
                        LibError::DlSym { .. } => {},
                        _ => {
                            eprintln!("[handle_post():{}] An unknown error occurred while executing the endpoint.\
                                                          Attempting to send Internal Server Error page to the client...", line!());
                            if let Err(e) = internal_server_error(stream).await {
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

        let mut set_cookie: HashMap<String, SetCookie> = HashMap::new();
        let content = endpoint("not_found", stream, Put { data, params }, headers, &mut response_headers, &mut set_cookie, &mut 404u16, library).await;
        let content_type = response_headers.get("content-type");

        if let (Ok(Some(c)), Some(c_t)) = (content, content_type) {
            let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                (mime.to_string(), mime.type_().to_string())
            } else {
                response_headers.remove(&String::from("content-type"));
                return send_response(stream, 404, Some(response_headers), None, Some(set_cookie), None).await;
            };

            if let Some(encoding) = CONFIG.get_response_encoding(&c, &mime_type, &general_type, headers) {
                response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
            }

            return send_response(stream, 404, Some(response_headers), Some(c), Some(set_cookie), Some(Dynamic)).await;
        }
        return send_response(stream, 404, Some(response_headers), None, Some(set_cookie), None).await;
    }

    response_headers = HashMap::from([
        (String::from("Accept"), format!("GET, HEAD, POST, OPTIONS{}", if CONFIG.enable_trace {", TRACE"} else {""}))
    ]);

    send_response(stream,405, Some(response_headers), None, None, None).await
}

pub async fn handle_delete<T>(stream: &mut T, headers: &HashMap<String, String>, mut resource: String, data: &Option<RequestBody>, params: &Option<HashMap<String, String>>) -> Result<(), Box<dyn Error>>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    let mut response_headers: HashMap<String, String> = HashMap::new();

    if let (Some(endpoints), Some(library)) = (&CONFIG.endpoints, &*ENDPOINT_LIBRARY) {
        resource.remove(0);

        if let Some(access_control) = &CONFIG.access_control {
            if !access_control.is_access_allowed(&resource) {
                let mut deny_action = access_control.deny_action;
                let mut set_cookie: HashMap<String, SetCookie> = HashMap::new();
                let content = endpoint(
                    if deny_action == 404 { "not_found" } else { "forbidden" },
                    stream,
                    Delete { data: &None, params: &None },
                    headers,
                    &mut response_headers,
                    &mut set_cookie,
                    &mut deny_action,
                    library).await;
                let content_type = response_headers.get("content-type");

                if let (Ok(Some(c)), Some(c_t)) = (content, content_type) {
                    let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                        (mime.to_string(), mime.type_().to_string())
                    } else {
                        response_headers.remove(&String::from("content-type"));
                        return send_response(stream, deny_action, Some(response_headers), None, Some(set_cookie), None).await;
                    };

                    if let Some(encoding) = CONFIG.get_response_encoding(&c, &mime_type, &general_type, headers) {
                        response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                        response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
                    }

                    return send_response(stream, deny_action, Some(response_headers), Some(c), Some(set_cookie), Some(Dynamic)).await;
                }
                return send_response(stream, deny_action, Some(response_headers), None, Some(set_cookie), None).await;
            }
        }

        if endpoints.contains(&resource) {
            let mut set_cookie: HashMap<String, SetCookie> = HashMap::new();
            let mut status: u16 = 200;
            let content = endpoint(&*resource, stream, Delete {data, params}, headers, &mut response_headers, &mut set_cookie, &mut status, library).await;
            let content_type = response_headers.get("content-type");

            match (content, content_type) {
                (Ok(Some(c)), Some(c_t)) => {
                    let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                        (mime.to_string(), mime.type_().to_string())
                    } else {
                        response_headers.remove(&String::from("content-type"));
                        return send_response(stream, status, Some(response_headers), None, Some(set_cookie), None).await;
                    };

                    if let Some(encoding) = CONFIG.get_response_encoding(&c, &mime_type, &general_type, headers) {
                        response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                        response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
                    }

                    if response_headers.contains_key("location") {
                        return send_response(stream, 302, Some(response_headers), Some(c), Some(set_cookie), Some(Dynamic)).await;
                    }

                    return send_response(stream, status, Some(response_headers), Some(c), Some(set_cookie), Some(Dynamic)).await;
                },
                (Ok(None), _) | (Ok(Some(_)), None) => {
                    if response_headers.contains_key("location") {
                        return send_response(stream, 302, Some(response_headers), None, Some(set_cookie), None).await;
                    }

                    return send_response(stream, status, Some(response_headers), None, Some(set_cookie), None).await;
                },
                (Err(e), _) => {
                    match e {
                        LibError::DlSym { .. } => {},
                        _ => {
                            eprintln!("[handle_post():{}] An unknown error occurred while executing the endpoint.\
                                                          Attempting to send Internal Server Error page to the client...", line!());
                            if let Err(e) = internal_server_error(stream).await {
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

        let mut set_cookie: HashMap<String, SetCookie> = HashMap::new();
        let content = endpoint("not_found", stream, Delete { data, params }, headers, &mut response_headers, &mut set_cookie, &mut 404u16, library).await;
        let content_type = response_headers.get("content-type");

        if let (Ok(Some(c)), Some(c_t)) = (content, content_type) {
            let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                (mime.to_string(), mime.type_().to_string())
            } else {
                response_headers.remove(&String::from("content-type"));
                return send_response(stream, 404, Some(response_headers), None, Some(set_cookie), None).await;
            };

            if let Some(encoding) = CONFIG.get_response_encoding(&c, &mime_type, &general_type, headers) {
                response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
            }

            return send_response(stream, 404, Some(response_headers), Some(c), Some(set_cookie), Some(Dynamic)).await;
        }
        return send_response(stream, 404, Some(response_headers), None, Some(set_cookie), None).await;
    }

    response_headers = HashMap::from([
        (String::from("Accept"), format!("GET, HEAD, POST, OPTIONS{}", if CONFIG.enable_trace {", TRACE"} else {""}))
    ]);

    send_response(stream,405, Some(response_headers), None, None, None).await
}

pub async fn handle_patch<T>(stream: &mut T, headers: &HashMap<String, String>, mut resource: String, data: &Option<RequestBody>, params: &Option<HashMap<String, String>>) -> Result<(), Box<dyn Error>>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    let mut response_headers: HashMap<String, String> = HashMap::new();

    if let (Some(endpoints), Some(library)) = (&CONFIG.endpoints, &*ENDPOINT_LIBRARY) {
        resource.remove(0);

        if let Some(access_control) = &CONFIG.access_control {
            if !access_control.is_access_allowed(&resource) {
                let mut deny_action = access_control.deny_action;
                let mut set_cookie: HashMap<String, SetCookie> = HashMap::new();
                let content = endpoint(
                    if deny_action == 404 { "not_found" } else { "forbidden" },
                    stream,
                    Patch { data: &None, params: &None },
                    headers,
                    &mut response_headers,
                    &mut set_cookie,
                    &mut deny_action,
                    library).await;
                let content_type = response_headers.get("content-type");

                if let (Ok(Some(c)), Some(c_t)) = (content, content_type) {
                    let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                        (mime.to_string(), mime.type_().to_string())
                    } else {
                        response_headers.remove(&String::from("content-type"));
                        return send_response(stream, deny_action, Some(response_headers), None, Some(set_cookie), None).await;
                    };

                    if let Some(encoding) = CONFIG.get_response_encoding(&c, &mime_type, &general_type, headers) {
                        response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                        response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
                    }

                    return send_response(stream, deny_action, Some(response_headers), Some(c), Some(set_cookie), Some(Dynamic)).await;
                }
                return send_response(stream, deny_action, Some(response_headers), None, Some(set_cookie), None).await;
            }
        }

        if endpoints.contains(&resource) {
            let mut set_cookie: HashMap<String, SetCookie> = HashMap::new();
            let mut status: u16 = 200;
            let content = endpoint(&*resource, stream, Patch {data, params}, headers, &mut response_headers, &mut set_cookie, &mut status, library).await;
            let content_type = response_headers.get("content-type");

            match (content, content_type) {
                (Ok(Some(c)), Some(c_t)) => {
                    let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                        (mime.to_string(), mime.type_().to_string())
                    } else {
                        response_headers.remove(&String::from("content-type"));
                        return send_response(stream, status, Some(response_headers), None, Some(set_cookie), None).await;
                    };

                    if let Some(encoding) = CONFIG.get_response_encoding(&c, &mime_type, &general_type, headers) {
                        response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                        response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
                    }

                    if response_headers.contains_key("location") {
                        return send_response(stream, 302, Some(response_headers), Some(c), Some(set_cookie), Some(Dynamic)).await;
                    }

                    return send_response(stream, status, Some(response_headers), Some(c), Some(set_cookie), Some(Dynamic)).await;
                },
                (Ok(None), _) | (Ok(Some(_)), None) => {
                    if response_headers.contains_key("location") {
                        return send_response(stream, 302, Some(response_headers), None, Some(set_cookie), None).await;
                    }

                    return send_response(stream, status, Some(response_headers), None, Some(set_cookie), None).await;
                },
                (Err(e), _) => {
                    match e {
                        LibError::DlSym { .. } => {},
                        _ => {
                            eprintln!("[handle_post():{}] An unknown error occurred while executing the endpoint.\
                                                          Attempting to send Internal Server Error page to the client...", line!());
                            if let Err(e) = internal_server_error(stream).await {
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

        let mut set_cookie: HashMap<String, SetCookie> = HashMap::new();
        let content = endpoint("not_found", stream, Patch { data, params }, headers, &mut response_headers, &mut set_cookie, &mut 404u16, library).await;
        let content_type = response_headers.get("content-type");

        if let (Ok(Some(c)), Some(c_t)) = (content, content_type) {
            let (mime_type, general_type) = if let Ok(mime) = Mime::from_str(c_t) {
                (mime.to_string(), mime.type_().to_string())
            } else {
                response_headers.remove(&String::from("content-type"));
                return send_response(stream, 404, Some(response_headers), None, Some(set_cookie), None).await;
            };

            if let Some(encoding) = CONFIG.get_response_encoding(&c, &mime_type, &general_type, headers) {
                response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
                response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
            }

            return send_response(stream, 404, Some(response_headers), Some(c), Some(set_cookie), Some(Dynamic)).await;
        }
        return send_response(stream, 404, Some(response_headers), None, Some(set_cookie), None).await;
    }

    response_headers = HashMap::from([
        (String::from("Accept"), format!("GET, HEAD, POST, OPTIONS{}", if CONFIG.enable_trace {", TRACE"} else {""}))
    ]);

    send_response(stream,405, Some(response_headers), None, None, None).await
}