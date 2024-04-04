use std::collections::HashMap;
use tokio::net::*;
use tokio::io::ErrorKind;
use sqlx::mysql::MySqlPoolOptions;
use crate::util::send_response;
use crate::config::CONFIG;
use crate::requests::RequestData;

pub async fn select(stream: &mut TcpStream, request: RequestData<'_>) -> Result<(), ErrorKind> {
    let maria_pool = MySqlPoolOptions::new()
        .connect(CONFIG.db_url)
        .await
        .unwrap();

    let records = sqlx::query!("SELECT * FROM customer")
        .fetch_all(&maria_pool)
        .await
        .unwrap();

    let mut content: String = String::from(
        r#"<!DOCTYPE html>
                <head>
                    <meta charset="utf-8">
                    <meta name="viewport" content="width=device-width, initial-scale=1.0">
                    <link rel="stylesheet" href="main.css">
                    <title>Formularz</title>
                </head>
                <body>
                    <table>
                        <tr>
                            <th>ID</th><th>Name</th><th>Phone</th><th>Address</th><th>City</th><th>State</th><th>Country</th><th>Zip code</th><th>Credit rating</th><th>Sales Representative ID</th><th>Region ID</th><th>Comments</th>
                        </tr>
                        "#
    );
    for x in records {
        content.push_str(&*format!(
        r#"             <tr>
                            <td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td>
                        </tr>"#, x.id,
            x.name,
            x.phone.unwrap_or("None".to_string()),
            x.address.unwrap_or("None".to_string()),
            x.city.unwrap_or("None".to_string()),
            x.state.unwrap_or("None".to_string()),
            x.country.unwrap_or("None".to_string()),
            x.zip_code.unwrap_or("None".to_string()),
            x.credit_rating.unwrap_or("None".to_string()),
            x.sales_rep_id.unwrap_or(0),
            x.region_id.unwrap_or(0),
            x.comments.unwrap_or("None".to_string())));
    }
    content.push_str(
        r#"       </table>
                </body>
            </html>"#);

    match request {
        RequestData::Get {..} => {
            return send_response(stream, 200, None, Some(content)).await
        },
        RequestData::Post {..} => {
            Ok(())
        },
        RequestData::Head {..} => {
            let content_length_string = content.len().to_string();
            let content_length_header = HashMap::from(("Content-Length", content_length_string.as_str()));
            return send_response(stream, 200, Some(content_length_header), None).await
        }
    }
}