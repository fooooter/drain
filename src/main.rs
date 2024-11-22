mod requests;
mod util;
mod pages;
mod config;
mod error;

use std::collections::HashMap;
use std::error::Error;
use tokio::net::*;
use tokio::*;
use crate::requests::Request::{Get, Head, Options, Post};
use crate::requests::*;
use crate::util::*;
use crate::config::config;
use crate::error::ServerError;

async fn handle_connection(mut stream: TcpStream) -> Result<(), Box<dyn Error>> {
    match receive_request(&mut stream).await {
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
            if let ServerError::DecompressionError(..) = e {
                send_response(&mut stream, 406, None, None, false).await?;
            } else {
                send_response(&mut stream, 400, None, None, false).await?;
            }
            Err(Box::new(e))
        }
    }
}

#[tokio::main]

async fn main() -> io::Result<()> {
    let listener = TcpListener::bind(config(None).await.bind).await?;
    loop {
        let (stream, _) = listener.accept().await?;
        spawn(async move {
            if let Err(e) = handle_connection(stream).await {
                eprintln!("[main():{}] An error occurred while handling connection:\n{}\n", line!(), e);
            }
        });
    }
}