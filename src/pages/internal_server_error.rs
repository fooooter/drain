use std::collections::HashMap;
use std::error::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use crate::util::send_response;

pub async fn internal_server_error<T>(mut stream: &mut T) -> Result<(), Box<dyn Error>>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    let content: Vec<u8> = Vec::from(format!(r#"
    <!DOCTYPE html>
    <html lang="en">
        <head>
            <meta charset="utf-8">
            <meta name="viewport" content="width=device-width, initial-scale=1.0">
            <title>500</title>
        </head>
        <body>
            <h2>500 Internal Server Error</h2>
        </body>
        <hr>
        <small>Drain {}</small>
    </html>
    "#, env!("CARGO_PKG_VERSION")));

    let response_headers = HashMap::from([(String::from("Content-Type"), String::from("text/html; charset=utf-8"))]);
    send_response(&mut stream, 500, Some(response_headers), Some(content), None).await
}