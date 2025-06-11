use std::collections::HashMap;
use std::error::Error;
use std::fs::read_dir;
use tokio::io::{AsyncRead, AsyncWrite};
use crate::config::CONFIG;
use crate::util::ResourceType::Dynamic;
use crate::util::send_response;
#[cfg(target_family = "unix")]
use crate::util::CHROOT;

pub async fn index_of<T>(mut stream: &mut T, directory: String, head: bool, headers: &HashMap<String, String>) -> Result<(), Box<dyn Error>>
where
    T: AsyncRead + AsyncWrite + Unpin
{
    #[cfg(target_family = "unix")]
    let document_root = if *&*CHROOT {&String::from("")} else {&CONFIG.document_root};
    #[cfg(not(target_family = "unix"))]
    let document_root = &CONFIG.document_root;

    let mut directory_list = String::new();

    match &CONFIG.access_control {
        Some(access_control) => {
            for dir in read_dir(format!("{document_root}/{directory}"))? {
                let dir = dir?;
                let path = dir.path();
                let path_str = String::from(path.to_string_lossy());
                let mut path_trim = path_str.trim_start_matches(document_root);
                path_trim = path_trim.trim_start_matches('/');

                if !access_control.is_access_allowed(&String::from(path_trim)) {
                    continue;
                }

                directory_list.push_str(&*format!("<li><a href=/{path_trim}>{path_trim}</a></li>"));
            }
        },
        _ => {
            for dir in read_dir(format!("{document_root}/{directory}"))? {
                let dir = dir?;
                let path = dir.path();
                let path_str = String::from(path.to_string_lossy());
                let mut path_trim = path_str.trim_start_matches(document_root);
                path_trim = path_trim.trim_start_matches('/');

                directory_list.push_str(&*format!("<li><a href=/{path_trim}>{path_trim}</a></li>"));
            }
        }
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

    if let Some(encoding) = CONFIG.get_response_encoding(&content, &String::from("text/html"), &String::from("text"), headers) {
        response_headers.insert(String::from("Content-Encoding"), String::from(encoding));
        response_headers.insert(String::from("Vary"), String::from("Accept-Encoding"));
    }

    if !head {
        return send_response(&mut stream, 200, Some(response_headers), Some(content), None, Some(Dynamic)).await;
    }
    response_headers.insert(String::from("Content-Length"), content.len().to_string());

    send_response(&mut stream, 200, Some(response_headers), None, None, None).await
}