<h3>Progress done so far (and TODO in a future):</h3>
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

<h4>This project aims to be similar to PHP/React.js, mainly in terms of dynamically generated web pages.</h4>

<p>Dynamic pages are generated inside a dynamic library, so that it's easy to create them without modifying<br>
the core and recompiling the server only to change one thing on a page. As of right now, such a page is <br>
partially hardcoded into the library, but I'm planning to make it loaded from a file as a template and <br>
processed using Handlebars to make it further isolated from the executable itself.</p>


