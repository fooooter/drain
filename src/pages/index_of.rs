use std::collections::HashMap;
use std::error::Error;
use std::fs::read_dir;
use tokio::io::{AsyncRead, AsyncWrite};
use crate::config::Config;
use crate::util::send_response;

pub async fn index_of<T>(mut stream: &mut T, config: &Config, mut directory: &String, head: bool) -> Result<(), Box<dyn Error>>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    directory.remove(0);
    let document_root = &config.document_root;

    let mut directory_list = String::new();
    for dir in read_dir(format!("{document_root}/{directory}"))? {
        let dir = dir?;
        let path = dir.path();
        let path_str = path.to_string_lossy();
        let path_split = path_str.split_once("/");
        let Some((_, path_str)) = path_split else {
            break;
        };

        if !config.is_access_allowed(&String::from(path_str), stream).await {
            continue;
        }

        directory_list.push_str(&*format!("<li><a href={path_str}>{path_str}</a></li>"));
    }

    let content: Vec<u8> = Vec::from(format!(r#"
    <!DOCTYPE html>
    <html lang="en">
        <head>
            <meta charset="utf-8">
            <meta name="viewport" content="width=device-width, initial-scale=1.0">
            <title>Index of /{directory}</title>
        </head>
        <body>
            <h2>Index of /{directory}</h2>

            <ul>
                {directory_list}
            </ul>
            <hr>
            <small>Drain {}</small>
        </body>
    </html>
    "#, env!("CARGO_PKG_VERSION")));

    let mut response_headers = HashMap::from([(String::from("Content-Type"), String::from("text/html; charset=utf-8"))]);

    if !head {
        return send_response(&mut stream, Some(config), 200, Some(response_headers), Some(content), None).await;
    }
    response_headers.insert(String::from("Content-Length"), content.len().to_string());
    send_response(&mut stream, Some(config), 200, Some(response_headers), None, None).await
}