In `crates/infrastructure/src/html.rs`, the function `has_remote_content` decides
whether an email contains remote content. Its result drives the "load images"
banner: when it returns true the user is offered a button to load the blocked
images, and when it returns false no banner is shown.

The function lowercases the HTML and then tests it against a fixed set of
literal substrings. Those literals all assume canonically formatted markup —
no unexpected whitespace inside an attribute assignment or inside a CSS
`url(...)`.

HTML in real email is not canonically formatted. Markup that a browser and the
sanitizer both treat as remote content can be written with extra whitespace in
places these literal checks do not account for. In that case
`has_remote_content` returns false while the sanitizer still strips the remote
resource.

The user-visible consequence: the image is removed from the message and no
banner appears, so there is no way to load it. The content is silently missing
with no indication that anything was blocked, and no recovery path.
