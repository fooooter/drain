use std::collections::HashMap;
use std::io::Read;
use chrono::Utc;
use flate2::Compression;
use flate2::read::GzEncoder;
use libloading::{Library};
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader, ErrorKind};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use bytes::BytesMut;
use crate::pages::internal_server_error::internal_server_error;
use crate::config::{config, get_config};
use crate::requests::{Request, RequestData};

pub async fn send_response(stream: &mut TcpStream, status: i32, local_response_headers: Option<HashMap<String, String>>, content: Option<String>, error: bool) -> Result<(), ErrorKind> {
    let mut response = String::new();
    let status_text = match status {
        100 => "Continue",
        101 => "Switching Protocols",
        102 => "Processing",
        103 => "Early Hints",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        203 => "Non-Authoritative Information",
        204 => "No Content",
        205 => "Reset Content",
        206 => "Partial Content",
        207 => "Multi-Status",
        208 => "Already Reported",
        226 => "IM Used",
        300 => "Multiple Choices",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        402 => "Payment Required",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        407 => "Proxy Authentication Required",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        411 => "Length Required",
        412 => "Precondition Failed",
        413 => "Content Too Large",
        414 => "URI Too Long",
        415 => "Unsupported Media Type",
        416 => "Range Not Satisfiable",
        417 => "Expectation Failed",
        418 => "I'm a teapot",
        421 => "Misdirected Request",
        422 => "Unprocessable Content",
        423 => "Locked",
        424 => "Failed Dependency",
        425 => "Too Early",
        426 => "Upgrade Required",
        428 => "Precondition Required",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        451 => "Unavailable For Legal Reasons",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        505 => "HTTP Version Not Supported",
        506 => "Variant Also Negotiates",
        507 => "Insufficient Storage",
        508 => "Loop Detected",
        510 => "Not Extended",
        511 => "Network Authentication Required",
        _ => return Err(ErrorKind::InvalidInput)
    };
    let status_line = format!("HTTP/1.1 {status} {status_text}\r\n");
    response.push_str(&*status_line);

    let date = get_current_date();

    let mut local_response_headers_clone = local_response_headers.clone();

    if let Some(ref mut h) = local_response_headers_clone {
        h.remove("Date");
    }

    let date_header = format!("Date: {}\r\n", date);
    response.push_str(&*date_header);

    let global_response_headers = if !error {
        Box::pin(get_config(Some(stream))).await.global_response_headers
    } else {
        HashMap::from([(String::from("Connection"), String::from("close"))])
    };
    let mut global_response_headers_clone = global_response_headers.clone();

    let mut response_bytes: Vec<u8>;

    match (local_response_headers_clone, content) {
        (Some(ref mut h), Some(c)) => {
            h.remove("Content-Length");

            for (k, _) in global_response_headers {
                if h.contains_key(&k) {
                    global_response_headers_clone.remove(&k);
                }
            }

            h.extend(global_response_headers_clone);

            for (k, v) in &mut *h {
                response.push_str(&*format!("{k}: {v}\r\n"));
            }

            let content_trim: &[u8] = c.trim().as_bytes();
            let mut content_prepared: Vec<u8> = Vec::new();

            if let Some(encoding) = h.get(&String::from("Content-Encoding")) {
                if encoding.eq("gzip") {
                    if let Err(e) = GzEncoder::new(content_trim, Compression::default()).read_to_end(&mut content_prepared) {
                        eprintln!("[send_response():{}] An error occurred while compressing the content of a response:\n{:?}\nAttempting to send uncompressed data...", line!(), e);
                        content_prepared = c.trim().as_bytes().to_vec();
                    }
                }
            }

            let content_length_header = format!("Content-Length: {}\r\n\r\n", content_prepared.len());
            response.push_str(&*format!("{content_length_header}"));

            response_bytes = response.as_bytes().to_vec();

            for x in content_prepared {
                response_bytes.push(x);
            }
        },
        (None, Some(c)) => {
            let content_length_header = format!("Content-Length: {}\r\n\r\n", c.len());
            response.push_str(&*format!("{content_length_header}{}", c.trim_start()));
            response_bytes = response.as_bytes().to_vec();
        },
        (Some(ref mut h), None) => {
            for (k, _) in global_response_headers {
                if h.contains_key(&k) {
                    global_response_headers_clone.remove(&k);
                }
            }

            h.extend(global_response_headers_clone);

            for (k, v) in h {
                response.push_str(&*format!("{k}: {v}\r\n"));
            }
            response.push_str("\r\n");
            response_bytes = response.as_bytes().to_vec();
        },
        (None, None) => {
            response.push_str("\r\n");
            response_bytes = response.as_bytes().to_vec();
        }
    }

    if let Err(e1) = stream.write_all(&*response_bytes).await {
        eprintln!("[send_response():{}] An error occurred while writing a response to a client:\n{:?}\nAttempting to close connection...", line!(), e1);
        if let Err(e2) = stream.shutdown().await {
            eprintln!("[receive_request():{}] FAILED. Error information:\n{:?}", line!(), e2);
        }
        panic!("Unrecoverable error occurred while handling connection.");
    }

    if let Err(e) = stream.flush().await {
        eprintln!("[send_response():{}] An error occurred while flushing the output stream:\n{:?}", line!(), e);
        return Err(ErrorKind::ConnectionAborted);
    }

    Ok(())
}

pub async fn receive_request(stream: &mut TcpStream) -> Result<Request, ErrorKind> {
    let mut reader = BufReader::new(&mut *stream);
    let mut request_string = String::new();

    loop {
        match reader.read_line(&mut request_string).await {
            Ok(l) => {
                if l == 2 {
                    break;
                }
            },
            Err(e1) => {
                eprintln!("[receive_request():{}] An error occurred while reading a request from a client. Error information:\n{:?}\nAttempting to close connection...", line!(), e1);
                if let Err(e2) = stream.shutdown().await {
                    eprintln!("[receive_request():{}] FAILED. Error information:\n{:?}", line!(), e2);
                }
                panic!("Unrecoverable error occurred while handling connection.");
            }
        };
    }

    let mut request = Request::parse_from_string(&request_string)?;

    if let  Request::Post {ref mut data, headers, ..} |
            Request::Put {ref mut data, headers, ..} |
            Request::Patch {ref mut data, headers, ..} = &mut request {
        let mut buffer = BytesMut::with_capacity(
    if let Ok(s) = headers.get("Content-Length").unwrap_or(&String::from("0")).parse::<usize>() {
                s
            } else {
                return Err(ErrorKind::InvalidInput);
            }
        );

        if let Err(e1) = reader.read_buf(&mut buffer).await {
            eprintln!("[receive_request():{}] An error occurred while reading a request from a client. Error information:\n{:?}\nAttempting to close connection...", line!(), e1);
            if let Err(e2) = stream.shutdown().await {
                eprintln!("[receive_request():{}] FAILED. Error information:\n{:?}", line!(), e2);
            }
            panic!("Unrecoverable error occurred while handling connection.");
        }

        let data_raw = String::from_utf8_lossy(&buffer).to_string();
        let mut data_hm: HashMap<String, String> = HashMap::new();

        for kv in data_raw.split('&') {
            if let Some(_) = &data_hm.insert(kv[..kv.find('=').unwrap()].to_string(), kv[kv.find('=').unwrap() + 1..].to_string()) {
                return Err(ErrorKind::InvalidInput);
            }
        }

        *data = Some(data_hm);
    }
    Ok(request)
}

pub async fn rts_wrapper(f: &mut File, buf: &mut String, stream: &mut TcpStream) {
    if let Err(e1) = f.read_to_string(buf).await {
        eprintln!("[rts_wrapper():{}] An error occurred after an attempt to read from a file: {:?}.\n\
                   Error information:\n\
                   {:?}\n\
                   Attempting to send Internal Server Error page to the client...", line!(), f, e1);
        if let Err(e2) = internal_server_error(stream).await {
            eprintln!("[rts_wrapper():{}] FAILED. Error information: {:?}", line!(), e2);
        }
        eprintln!("Attempting to close connection...");
        if let Err(e2) = stream.shutdown().await {
            eprintln!("[rts_wrapper():{}] FAILED. Error information:\n{:?}", line!(), e2);
        }
        panic!("Unrecoverable error occurred while handling connection.");
    }
}

pub fn get_current_date() -> String {
    let dt = Utc::now();
    let dt_formatted = dt.format("%a, %e %b %Y %T GMT");
    dt_formatted.to_string()
}

pub fn accepts_gzip(headers: &HashMap<String, String>) -> bool {
    if let Some(encodings_str) = headers.get("Accept-Encoding") {
        let encodings: Vec<String> = encodings_str.split(',').map(|x| String::from(x.trim())).collect();

        if encodings.contains(&String::from("gzip")) {
            return true;
        }
        false
    } else {
        false
    }
}

pub async fn page<'a>(page: &str, stream: &mut TcpStream, request_data: RequestData<'a>, mut response_headers: &mut HashMap<String, String>) -> Result<String, ErrorKind> {
    unsafe {
        let lib = if let Ok(l) = Library::new(config(Some(stream)).await.dynamic_pages_library) {
            l
        } else {
            return Err(ErrorKind::Other);
        };

        let p = if let Ok(s) = lib.get::<fn(RequestData, &mut HashMap<String, String>) -> String>(page.as_bytes()) {
            s
        } else {
            return Err(ErrorKind::NotFound);
        };

        Ok(p(request_data, &mut response_headers))
    }
}