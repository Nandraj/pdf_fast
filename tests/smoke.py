"""Smoke test run in CI against the freshly built wheel (no third-party deps)."""
import os
import tempfile

import pdf_fast


def build_pdf(n_pages: int) -> bytes:
    """Hand-build a minimal valid PDF with n_pages pages."""
    objs = {}
    kids = " ".join(f"{3 + i} 0 R" for i in range(n_pages))
    objs[1] = "<</Type/Catalog/Pages 2 0 R>>"
    objs[2] = f"<</Type/Pages/Count {n_pages}/Kids[{kids}]/MediaBox[0 0 200 200]>>"
    font_id = 3 + 2 * n_pages
    for i in range(n_pages):
        page_id, content_id = 3 + i, 3 + n_pages + i
        objs[page_id] = (
            f"<</Type/Page/Parent 2 0 R/Contents {content_id} 0 R"
            f"/Resources<</Font<</F1 {font_id} 0 R>>>>>>"
        )
        body = f"BT /F1 12 Tf 10 100 Td (Page {i + 1}) Tj ET"
        objs[content_id] = f"<</Length {len(body)}>>\nstream\n{body}\nendstream"
    objs[font_id] = "<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>"

    out = bytearray(b"%PDF-1.4\n")
    offsets = {}
    for num in sorted(objs):
        offsets[num] = len(out)
        out += f"{num} 0 obj\n{objs[num]}\nendobj\n".encode()
    xref = len(out)
    size = max(objs) + 1
    out += f"xref\n0 {size}\n0000000000 65535 f \n".encode()
    for num in range(1, size):
        out += f"{offsets[num]:010} 00000 n \n".encode()
    out += f"trailer\n<</Size {size}/Root 1 0 R>>\nstartxref\n{xref}\n%%EOF\n".encode()
    return bytes(out)


def main() -> None:
    with tempfile.TemporaryDirectory() as d:
        src = os.path.join(d, "src.pdf")
        dst = os.path.join(d, "dst.pdf")
        with open(src, "wb") as f:
            f.write(build_pdf(5))

        assert pdf_fast.page_count(src) == 5

        assert pdf_fast.extract_first_pages(src, dst, 2) == 2
        assert pdf_fast.page_count(dst) == 2

        # n larger than the document returns everything
        assert pdf_fast.extract_first_pages(src, dst, 99) == 5
        assert pdf_fast.page_count(dst) == 5

        # bad input raises our exception, never crashes the interpreter
        bad = os.path.join(d, "bad.pdf")
        with open(bad, "wb") as f:
            f.write(b"%PDF-1.4\ngarbage")
        try:
            pdf_fast.page_count(bad)
        except pdf_fast.PdfFastError:
            pass
        else:
            raise AssertionError("expected PdfFastError")

        try:
            pdf_fast.extract_first_pages(src, dst, 0)
        except ValueError:
            pass
        else:
            raise AssertionError("expected ValueError for n=0")

    print("smoke test passed")


if __name__ == "__main__":
    main()
