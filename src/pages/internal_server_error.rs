use std::collections::HashMap;
use std::io::ErrorKind;
use tokio::net::TcpStream;
use crate::util::send_response;

pub async fn internal_server_error(mut stream: &mut TcpStream) -> Result<(), ErrorKind> {
    let content = String::from(r#"
    <!DOCTYPE html>
    <html lang="en">
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

    let response_headers = HashMap::from([(String::from("Content-Type"), String::from("text/html; charset=utf-8"))]);

    send_response(&mut stream, 500, Some(response_headers), Some(content), true).await
}