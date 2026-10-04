import os, tempfile
from flask import Flask, request, jsonify, send_file, abort
import pdf_fast

app = Flask(__name__)
app.config["MAX_CONTENT_LENGTH"] = 200 * 1024 * 1024
MAX_PAGES = 200

def save_upload(fs):
    fd, path = tempfile.mkstemp(suffix=".pdf")
    os.close(fd)
    fs.save(path)                      # streams to disk, not RAM
    return path

@app.post("/pdf/count")
def count():
    f = request.files.get("file") or abort(400, "file required")
    path = save_upload(f)
    try:
        return jsonify(pages=pdf_fast.page_count(path))
    except pdf_fast.PdfFastError as ex:
        abort(422, str(ex))
    finally:
        os.remove(path)

@app.post("/pdf/first-pages")
def first_pages():
    f = request.files.get("file") or abort(400, "file required")
    n = min(request.form.get("n", 1, type=int), MAX_PAGES)
    src = save_upload(f)
    dst = src + ".out.pdf"
    try:
        pdf_fast.extract_first_pages(src, dst, n)
    except (pdf_fast.PdfFastError, ValueError) as ex:
        abort(422, str(ex))
    finally:
        os.remove(src)
    resp = send_file(dst, mimetype="application/pdf", download_name="preview.pdf")
    resp.call_on_close(lambda: os.path.exists(dst) and os.remove(dst))
    return resp
