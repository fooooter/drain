use std::collections::HashMap;
use chrono::Utc;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader, ErrorKind};
use tokio::io::AsyncWriteExt;
use tokio::net::*;
use crate::pages::internal_server_error::internal_server_error;

pub async fn send_response(stream: &mut TcpStream, status: i32, headers: Option<HashMap<&str, &str>>, content: Option<String>) -> Result<(), ErrorKind> {
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

    match (headers, content) {
        (Some(h), Some(s)) => {
            let mut headers_clone = h.clone();
            headers_clone.remove("Content-Length");

            for (k, v) in headers_clone {
                response.push_str(&*format!("{k}: {v}\r\n"));
            }

            let content_length_header = format!("Content-Length: {}\r\n\r\n", s.len());
            response.push_str(&*format!("{content_length_header}{s}"));
        },
        (None, Some(s)) => {
            let content_length_header = format!("Content-Length: {}\r\n\r\n", s.len());
            response.push_str(&*format!("{content_length_header}{s}"));
        },
        (Some(h), None) => {
            let mut headers_clone = h.clone();

            for (k, v) in headers_clone {
                response.push_str(&*format!("{k}: {v}\r\n"));
            }
        },
        _ => {}
    }

    if let Err(e1) = stream.write_all(response.as_bytes()).await {
        eprintln!("[send_response():89] An error occurred while writing a response to a client:\n{:?}\nAttempting to close connection...", e1);
        if let Err(e2) = stream.shutdown().await {
            eprintln!("[send_response():91] Clean shutdown failed:\n{:?}", e2);
            panic!("Unrecoverable errors occurred while handling connection:\n{e1}\n{e2}");
        }
        panic!("Unrecoverable error occurred while handling connection:\n{e1}");
    }
    Ok(())
}

pub async fn receive_request(mut stream: &mut TcpStream, request: &mut String) {
    let reader = BufReader::new(&mut stream);
    let mut request_iter = reader.lines();

    let mut line = match request_iter.next_line().await {
        Ok(line) => line.unwrap(),
        Err(e1) => {
            eprintln!("[receive_request():103] An error occurred while reading a request from a client. Error information:\n{:?}\nAttempting to close connection...", e1);
            if let Err(e2) = stream.shutdown().await {
                eprintln!("[receive_request():123] FAILED. Error information:\n{:?}", e2);
            }
            panic!("Unrecoverable error occurred while handling a connection.");
        }
    };

    while !line.is_empty() {
        (*request).push_str(format!("{line}\r\n").as_str());
        line = match request_iter.next_line().await {
            Ok(line) => line.unwrap(),
            Err(e1) => {
                eprintln!("[receive_request():119] An error occurred while reading a request from a client. Error information:\n{:?}\nAttempting to close connection...", e1);
                if let Err(e2) = stream.shutdown().await {
                    eprintln!("[receive_request():123] FAILED. Error information:\n{:?}", e2);
                }
                panic!("Unrecoverable error occurred while handling connection.");
            }
        };
    }
}

pub async fn read_to_string_wrapper(f: &mut File, buf: &mut String, stream: &mut TcpStream, headers: &HashMap<String, String>) {
    if let Err(e1) = f.read_to_string(buf).await {
        eprintln!("[not_found():11] An error occurred after an attempt to read from a file: {:?}.\n\
                           Error information:\n\
                           {:?}\n\
                           Attempting to send Internal Server Error page to the client...", f, e1);
        if let Err(e2) = internal_server_error(stream, headers).await {
            eprintln!("[not_found():19] FAILED. Error information: {:?}", e2);
        }
        eprintln!("Attempting to close connection...");
        if let Err(e2) = stream.shutdown().await {
            eprintln!("[not_found():22] FAILED. Error information:\n{:?}", e2);
        }
        panic!("Unrecoverable error occurred while handling connection.");
    }
}

pub fn get_current_date() -> String {
    let dt = Utc::now();
    let dt_formatted = dt.format("%a, %e %b %Y %T GMT");
    dt_formatted.to_string()
}