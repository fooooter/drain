use std::collections::HashMap;
use crate::RequestData;

#[no_mangle]
pub fn index(request_data: RequestData, mut response_headers: &mut HashMap<String, String>) -> String {
    let content = String::from(r#"
    <!DOCTYPE html>
        <head>
            <meta charset="utf-8">
            <meta name="viewport" content="width=device-width, initial-scale=1.0">
            <link rel="stylesheet" href="main.css">
            <title>Index</title>
        </head>
        <body>
            Hello, world!
        </body>
    </html>"#
    );

    response_headers.insert(String::from("Content-Type"), String::from("text/html; charset=utf-8"));

    content
}