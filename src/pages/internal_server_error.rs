use std::collections::HashMap;
use std::error::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use crate::util::send_response;

pub async fn internal_server_error<T>(mut stream: &mut T) -> Result<(), Box<dyn Error>>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    let content: Vec<u8> = Vec::from(include_str!("internal_server_error.html"));

    let response_headers = HashMap::from([(String::from("Content-Type"), String::from("text/html; charset=utf-8"))]);
    send_response(&mut stream, None, 500, Some(response_headers), Some(content), None).await
}