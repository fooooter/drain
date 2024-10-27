use std::collections::HashMap;
use tokio::net::*;
use tokio::io::ErrorKind;
use crate::util::send_response;

pub async fn not_found(mut stream: &mut TcpStream, _headers: &HashMap<String, String>) -> Result<(), ErrorKind> {
    let content = String::from(r#"
    <!DOCTYPE html>
        <head>
            <meta charset="utf-8">
            <meta name="viewport" content="width=device-width, initial-scale=1.0">
            <link rel="stylesheet" href="main.css">
            <title>404</title>
        </head>
        <body>
            Requested content isn't found on the server.
        </body>
    </html>"#
    );

    let content_type_header = HashMap::from([(String::from("Content-Type"), String::from("text/html; charset=utf-8"))]);

    send_response(&mut stream, 404, Some(content_type_header), Some(content), false).await
}