use std::collections::HashMap;
use tokio::net::*;
use tokio::io::ErrorKind;
use crate::requests::RequestData;
use crate::util::send_response;

pub async fn not_found<'a>(mut stream: &mut TcpStream, request_data: RequestData<'a>, mut response_headers: HashMap<String, String>) -> Result<(), ErrorKind> {
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

    response_headers.insert(String::from("Content-Type"), String::from("text/html; charset=utf-8"));

    send_response(&mut stream, 404, Some(response_headers), Some(content), false).await
}