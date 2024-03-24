use std::collections::HashMap;
use tokio::net::*;
use tokio::io::{AsyncWriteExt, ErrorKind};
use tokio::fs::File;
use crate::pages::internal_server_error::internal_server_error;
use crate::util::{read_to_string_wrapper, send_response, get_current_date};

pub async fn not_found(mut stream: &mut TcpStream, headers: &HashMap<String, String>) -> Result<(), ErrorKind> {
    let mut content = String::new();
    let webpage_404 = File::open("404").await;
    match webpage_404 {
        Ok(mut f) => {
            read_to_string_wrapper(&mut f, &mut content, stream, headers).await;

            let date = get_current_date();
            let response_headers = HashMap::from([
                // ("Server", "stary najebany"),
                ("Connection", "keep-alive"),
                ("Keep-Alive", "timeout=5, max=100"),
                ("Date", date.as_str()),
                ("Content-Type", "text/html; charset=utf-8")]);

            if let Err(e) = send_response(&mut stream, 404, Some(response_headers), Some(content)).await {
                return Err(e)
            }
            Ok(())
        },
        Err(e) => {
            eprintln!("[not_found():11] An error occurred after an attempt to open a file. Error information:\n\
            {:?}\n\
            Attempting to send Internal Server Error page to the client...", e);
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
}