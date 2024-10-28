mod requests;
mod util;
mod pages;
mod config;

use std::collections::HashMap;
use tokio::net::*;
use tokio::*;
use tokio::io::ErrorKind;
use requests::Request::{Get, Head, Options, Post};
use requests::*;
use crate::util::*;

async fn handle_connection(mut stream: TcpStream) -> Result<(), ErrorKind> {
    let mut request_string = String::new();

    receive_request(&mut stream, &mut request_string).await;

    match Request::parse_from_string(&request_string) {
        Ok(request) => {
            match request {
                Get {resource, params, headers} => handle_get(stream, &headers, &resource, &params).await,
                Head {resource, headers} => handle_head(stream, &headers, &resource).await,
                Post {resource, headers, data} => handle_post(stream, &headers, &resource, &data).await,
                Options {resource, headers} => handle_options(stream, &headers, &resource).await,
                _ => {
                    let accept_header = HashMap::from([(String::from("Accept"), String::from("GET, HEAD, POST, OPTIONS"))]);

                    send_response(&mut stream, 405, Some(accept_header), None, false).await
                }
            }
        },
        Err(e) => {
            send_response(&mut stream, 400, None, None, false).await?;
            Err(e)
        }
    }
}

#[tokio::main]

async fn main() -> io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:8080").await?;
    loop {
        let (stream, _) = listener.accept().await?;
        spawn(async move {
            handle_connection(stream).await
        });
    }
}


