# Canvas content protocol

The assistant exposes `canvas.show` as a local, schema-validated tool. Its arguments
contain a `kind` and `title`, plus the fields for that kind:

- `text`: a `text` string, up to 8 KiB.
- `table`: `columns` and `rows` arrays of strings, up to 12 columns and 50 rows.
- `image`: a local `path` to a PNG, JPEG, WebP, GIF or BMP image.
- `gallery`: a `paths` array of up to 12 local images.
- `chart`: up to 50 `{label, value}` pairs for a nonnegative bar chart.
- `line_chart`: two to 200 `{x, y}` numeric points, plotted in order.
- `pdf`: a local PDF `path` up to 128 MiB, shown one page at a time.
- `document`: a local UTF-8 text document `path` up to 1 MiB.
- `map`: `latitude`, `longitude`, and optional `zoom`. Map tiles are requested from OpenStreetMap when the map is shown.
- `video`: a local MP4, M4V, WebM, MKV, MOV, AVI, MPEG or OGV `path`. Playback starts paused and uses the shared Media Player controls.
- `links`: an optional `text` and up to 20 `{label, url}` HTTP(S) links. Links are opened only when clicked.

Calls use the existing `files_read` permission and the assistant's HOME and secret-path
guards. An image must be a regular file no larger than 32 MiB. The tool returns a `structuredContent.canvas`
object with protocol `version: 1`, which Canvas handles from the assistant's tool event.
The complete content is limited to 12,000 bytes so it fits the event limit. URLs are not
loaded as images. Canvas does not open for ordinary speech.

Canvas retains a pinned page when new content arrives and places follow-up content on a
new page. Search results use the existing search event. A video pauses and releases its
player when its page is hidden; selecting the page again reloads it at its last position.
