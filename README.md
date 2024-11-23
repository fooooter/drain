### Progress done so far (and TODO in the future):
[✔]   	GET<br>
[✔]   	OPTIONS<br>
[✔]   	HEAD<br>
[✔]   	POST<br>
[✖]   	Database connection<br>
[✔]	Auto-detect MIME types<br>
[✖]		Cookies<br>
[✖]		Sessions<br>
[✔]	Config (now JSON)<br>
[✔]     Compression (GZIP and Brotli for now)<br>
[✔]     Decompression (GZIP and Brotli for now)

### This project aims to be similar to PHP/React.js, mainly in terms of dynamically generated web pages.

Dynamic pages are generated inside a dynamic library, so that it's easy to create them without modifying
the core and recompiling the server only to change one thing on a page. As of right now, such a page is
partially hardcoded into the library, but I'm planning to make it loaded from a file as a template and
processed using Handlebars to make it further isolated from the executable itself.

### Build

- To build the server, run `cargo build` in the root of a source.
- To build the library containing the dynamic pages, run `cargo build` in dynamic_pages directory (don't forget to specify the binary in config.json)

### Configuration

Server can be configured using config.json file. Currently available fields are:

- `global_response_headers` - it's a list of key-value pairs, which stand for default response headers appended to every
`response_headers` HashMap
- `access_control` - here you can control, which resources will be returned to the client and which won't through a list
of key-value pairs. In order to deny access to a resource, type "deny" (default action is "allow"). If you deny access,
the client will get a 404 error, but this will be changeable in the future. It uses Glob UNIX shell-like path syntax.
- `bind` - bind host and port to the server.
- `dynamic_pages` - holds a list of every dynamic page available, so if you create one, be sure to specify it here!
- `dynamic_pages_library` - a path to the dynamic library for dynamic pages.
- `supported_encodings` - a list of all compression algorithms supported by the server. It can currently contain only "gzip" and "br".
- `use_encoding` - a name of encoding which will be used to compress the response body. It should match the `supported_encodings` field.

Changing the `bind` field requires restarting the server for it to take effect.

### Usage

Each page should be a Rust module defined in a separate file, declared in lib.rs and have the following structure:

```rust
use std::collections::HashMap;
use crate::RequestData::{*, self};

#[no_mangle]
pub fn example(request_data: RequestData, response_headers: &mut HashMap<String, String>) -> String {
    let content = String::from(r#"
    <!DOCTYPE html>
        <head>
            <meta charset="utf-8">
            <meta name="viewport" content="width=device-width, initial-scale=1.0">
            <link rel="stylesheet" href="main.css">
            <title>Example</title>
        </head>
        <body>
            Hello, world!
        </body>
    </html>"#
    );

    response_headers.insert(String::from("Content-Type"), String::from("text/html; charset=utf-8"));

    content
}
```

`RequestData` is a struct-like Enum, which has variants, that tell, what kind of HTTP request method was used and stores
request headers and data specific to each variant.

```rust
pub enum RequestData<'a> {
    Get {params: &'a Option<HashMap<String, String>>, headers: &'a HashMap<String, String>},
    Post {headers: &'a HashMap<String, String>, data: &'a Option<HashMap<String, String>>},
    Head {headers: &'a HashMap<String, String>}
}
```

POST "data" is an application/x-www-form-urlencoded string parsed to a HashMap and GET
"params" are regular key-value pairs sent in the URL.

`response_headers` is a HashMap containing every header, that will be sent in response. It's a mutable reference,
so that you can simply append a header to existing ones. Its best use cases are redirections using `Location` header and
changing content type to JSON, for example.