use std::collections::HashMap;
use std::error::Error;
use tokio::net::TcpStream;
use crate::util::send_response;

pub async fn internal_server_error(mut stream: &mut TcpStream) -> Result<(), Box<dyn Error>> {
    let content = String::from(include_str!("internal_server_error.html"));

    let response_headers = HashMap::from([(String::from("Content-Type"), String::from("text/html; charset=utf-8"))]);
    send_response(&mut stream, 500, Some(response_headers), Some(content), true).await
}