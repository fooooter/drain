use std::collections::HashMap;
use std::io::ErrorKind;
use tokio::net::TcpStream;
use crate::util::{send_response, get_current_date};

pub async fn internal_server_error(mut stream: &mut TcpStream, headers: &HashMap<String, String>) -> Result<(), ErrorKind> {
    let content = String::from(
r#"<html lang="pl">
        <head>
            <meta charset="utf-8">
            <meta name="viewport" content="width=device-width, initial-scale=1.0">
            <link rel="stylesheet" href="main.css">
            <title>500</title>
        </head>
        <body>
            <h2>500 Internal Server Error</h2>
        </body>
    </html>"#
    );

    let date = get_current_date();
    let response_headers = HashMap::from([
        ("Server", "stary najebany"),
        ("Connection", "keep-alive"),
        ("Keep-Alive", "timeout=5, max=100"),
        ("Date", date.as_str()),
        ("Content-Type", "text/html; charset=utf-8")]);

    if let Err(e) = send_response(&mut stream, 500, Some(response_headers), Some(content)).await {
        return Err(e)
    }
    Ok(())
}