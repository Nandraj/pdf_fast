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
