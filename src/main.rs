mod requests;
mod util;
mod pages;
mod config;
mod error;

use std::collections::HashMap;
use std::error::Error;

use std::env::set_current_dir;
use tokio::net::*;
use tokio::*;
use crate::requests::Request::{Get, Head, Options, Post};
use crate::requests::*;
use crate::util::*;
use crate::config::Config;
use crate::error::ServerError;

async fn handle_connection(mut stream: TcpStream) -> Result<(), Box<dyn Error>> {
    let config = Config::new(Some(&mut stream)).await;

    match receive_request(&mut stream, &config).await {
        Ok(request) => {
            match request {
                Get {resource, params, headers} => handle_get(stream, config, &headers, resource, &params).await,
                Head {resource, headers} => handle_head(stream, config, &headers, resource).await,
                Post {resource, headers, data} => handle_post(stream, config, &headers, resource, &data).await,
                Options {resource, headers} => handle_options(stream, config, &headers, resource).await,
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
    let config = Config::new(None).await;
    let document_root = &config.document_root;

    set_current_dir(document_root)?;

    let listener = TcpListener::bind(config.bind).await?;
    loop {
        let (stream, _) = listener.accept().await?;
        spawn(async move {
            if let Err(e) = handle_connection(stream).await {
                eprintln!("[main():{}] An error occurred while handling connection:\n{e}\n", line!());
            }
        });
    }
}