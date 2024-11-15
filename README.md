## Progress done so far (and TODO in the future):
[✔]   	GET<br>
[✔]   	OPTIONS<br>
[✔]   	HEAD<br>
[✔]   	POST<br>
[✖]   	Database connection<br>
[✔]	Auto-detect MIME types<br>
[✖]		Cookies<br>
[✖]		Sessions<br>
[✔]	Config (now JSON)<br>
[✔]	Compression<br>

### This project aims to be similar to PHP/React.js, mainly in terms of dynamically generated web pages.

Dynamic pages are generated inside a dynamic library, so that it's easy to create them without modifying
the core and recompiling the server only to change one thing on a page. As of right now, such a page is
partially hardcoded into the library, but I'm planning to make it loaded from a file as a template and
processed using Handlebars to make it further isolated from the executable itself.

### Build

- To build the server, run `cargo build` in the root of a source.
- To build the library containing the dynamic pages, run `cargo build` (don't forget to specify the binary in config.json)