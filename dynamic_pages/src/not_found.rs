use std::collections::HashMap;
use crate::RequestData;

#[export_name = "not_found"]
pub fn not_found(request_data: RequestData, mut response_headers: &mut HashMap<String, String>) -> Option<String> {
    let content = String::from(r#"
    <!DOCTYPE html>
    <html lang="en">
        <head>
            <meta charset="utf-8">
            <meta name="viewport" content="width=device-width, initial-scale=1.0">
            <title>404</title>
        </head>
        <body>
            <h2>404 Not Found</h2>
        </body>
    </html>"#
    );

    response_headers.insert(String::from("Content-Type"), String::from("text/html; charset=utf-8"));

    Some(content)
}