# pdf-fast

Fast, low-memory PDF **page count** and **first-N-pages extraction**, written in Rust.

```python
import pdf_fast

pdf_fast.page_count("big.pdf")                       # -> 1832
pdf_fast.extract_first_pages("big.pdf", "out.pdf", 5) # -> 5 (pages written)
```

* The source file is memory-mapped, never read into RAM.
* Page count touches only the xref, trailer and page-tree root.
* Extraction copies only objects reachable from the first N pages.
* The GIL is released while working, so Flask/gunicorn threads run in parallel.
* Raises `pdf_fast.PdfFastError` on corrupt or password-protected files
  (pass `password=b"..."` to open encrypted ones; output is written decrypted).

## Options

```python
pdf_fast.page_count(path, password=None, verify=False, timeout=None)
pdf_fast.extract_first_pages(src, dst, n, password=None, max_bytes=None, timeout=None)
```

* `verify=True` counts real pages by walking the page tree instead of trusting
  the file's `/Count` (still milliseconds, but not O(1)).
* `max_bytes` caps output size and rejects oversized streams before reading them.
* `timeout` (seconds) is checked between objects; it cannot interrupt a single parse.

## Caveats

* The input is memory-mapped: do **not** truncate or overwrite a file while a call
  is running (the process receives SIGBUS). Pass files you exclusively own,
  e.g. a fresh upload temp file.
* Files with a damaged xref table are rejected rather than repaired; fall back to
  PyMuPDF/pikepdf on `PdfFastError` if you need to accept those.
