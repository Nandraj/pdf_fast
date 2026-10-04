//! pdf_fast: page count + first-N-pages extraction without loading the file.
//!
//! * The input is memory-mapped; the OS pages in only what we touch.
//! * Page count reads the xref + trailer + page-tree root only.
//! * Extraction walks the page tree lazily, then copies *only* the objects
//!   reachable from the selected pages straight to the output file.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use memmap2::Mmap;
use pdf::file::FileOptions;
use pdf::object::{PlainRef, Resolve};
use pdf::primitive::{Dictionary, Primitive};
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyValueError};
use pyo3::prelude::*;

create_exception!(pdf_fast, PdfFastError, PyException, "Raised for unreadable, encrypted or corrupt PDFs.");

type Res<T> = Result<T, String>;

/// The pdf crate emits long "caused by" chains; keep only the root cause.
fn e<E: std::fmt::Display>(err: E) -> String {
    let s = err.to_string();
    let last = s.lines().map(str::trim).rev().find(|l| !l.is_empty()).unwrap_or("unknown error");
    format!("invalid or unsupported PDF: {last}")
}

fn map_file(path: &Path) -> Res<Mmap> {
    let f = File::open(path).map_err(|x| format!("cannot open {}: {x}", path.display()))?;
    let len = f.metadata().map_err(e)?.len();
    if len == 0 {
        return Err("empty file".into());
    }
    // SAFETY: the file must not be truncated by another process while mapped.
    unsafe { Mmap::map(&f) }.map_err(e)
}

// ---------------------------------------------------------------- page count

fn page_count_impl(path: &Path, password: &[u8]) -> Res<u32> {
    let mmap = map_file(path)?;
    let file = FileOptions::uncached().password(password).load(mmap).map_err(e)?;
    Ok(file.num_pages())
}

// ---------------------------------------------------------------- extraction

const INHERITABLE: [&str; 4] = ["Resources", "MediaBox", "CropBox", "Rotate"];

fn type_is(d: &Dictionary, names: &[&str]) -> bool {
    matches!(d.get("Type"), Some(Primitive::Name(n)) if names.contains(&n.as_str()))
}

fn as_dict(p: Primitive) -> Option<Dictionary> {
    match p {
        Primitive::Dictionary(d) => Some(d),
        Primitive::Stream(s) => Some(s.info),
        _ => None,
    }
}

/// Depth-first walk of the page tree that stops after `limit` leaves and
/// pushes inherited attributes down into each leaf.
fn collect_pages<R: Resolve>(
    r: &R,
    node: PlainRef,
    inherited: &Dictionary,
    out: &mut Vec<(PlainRef, Dictionary)>,
    limit: usize,
    seen: &mut HashSet<(u64, u64)>,
    depth: usize,
) -> Res<()> {
    if out.len() >= limit || depth > 64 || !seen.insert((node.id as u64, node.gen as u64)) {
        return Ok(());
    }
    let dict = as_dict(r.resolve(node).map_err(e)?).ok_or("page tree node is not a dictionary")?;

    let mut inh = inherited.clone();
    for k in INHERITABLE {
        if let Some(v) = dict.get(k) {
            inh.insert(k, v.clone());
        }
    }

    match dict.get("Kids") {
        Some(Primitive::Array(kids)) if !type_is(&dict, &["Page"]) => {
            for kid in kids {
                if let Primitive::Reference(kr) = kid {
                    collect_pages(r, *kr, &inh, out, limit, seen, depth + 1)?;
                    if out.len() >= limit {
                        break;
                    }
                }
            }
        }
        _ => {
            let mut page = dict;
            for k in INHERITABLE {
                if page.get(k).is_none() {
                    if let Some(v) = inh.get(k) {
                        page.insert(k, v.clone());
                    }
                }
            }
            page.remove("Parent");
            out.push((node, page));
        }
    }
    Ok(())
}

// ---- serializer (direct objects only; refs were already remapped) ----

fn write_name<W: Write>(s: &str, out: &mut W) -> io::Result<()> {
    out.write_all(b"/")?;
    for &b in s.as_bytes() {
        let regular = (0x21..=0x7e).contains(&b) && !b"()<>[]{}/%#".contains(&b);
        if regular {
            out.write_all(&[b])?;
        } else {
            write!(out, "#{b:02X}")?;
        }
    }
    Ok(())
}

fn write_prim<W: Write>(p: &Primitive, out: &mut W) -> io::Result<()> {
    match p {
        Primitive::Null => out.write_all(b"null"),
        Primitive::Integer(i) => write!(out, "{i}"),
        Primitive::Number(f) => {
            if f.is_finite() { write!(out, "{f}") } else { out.write_all(b"0") }
        }
        Primitive::Boolean(b) => write!(out, "{b}"),
        Primitive::String(s) => {
            out.write_all(b"<")?;
            for b in s.as_bytes() {
                write!(out, "{b:02X}")?;
            }
            out.write_all(b">")
        }
        Primitive::Name(n) => write_name(n.as_str(), out),
        Primitive::Array(a) => {
            out.write_all(b"[")?;
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.write_all(b" ")?;
                }
                write_prim(x, out)?;
            }
            out.write_all(b"]")
        }
        Primitive::Dictionary(d) => write_dict(d, out),
        Primitive::Reference(r) => write!(out, "{} {} R", r.id, r.gen),
        Primitive::Stream(_) => out.write_all(b"null"), // streams are never direct objects
    }
}

fn write_dict<W: Write>(d: &Dictionary, out: &mut W) -> io::Result<()> {
    out.write_all(b"<<")?;
    for (k, v) in d.iter() {
        write_name(k.as_str(), out)?;
        out.write_all(b" ")?;
        write_prim(v, out)?;
        out.write_all(b"\n")?;
    }
    out.write_all(b">>")
}

struct Counter<W: Write> {
    inner: W,
    pos: u64,
}
impl<W: Write> Write for Counter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.pos += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

// ---- object copier: renumbers refs lazily, BFS, only reachable objects ----

struct Copier<'a, R: Resolve> {
    r: &'a R,
    map: HashMap<(u64, u64), u32>,
    queue: VecDeque<(u32, Primitive)>,
    next_id: u32,
}

impl<'a, R: Resolve> Copier<'a, R> {
    fn map_ref(&mut self, old: PlainRef) -> Primitive {
        let key = (old.id as u64, old.gen as u64);
        if let Some(&id) = self.map.get(&key) {
            return Primitive::Reference(PlainRef { id: id as _, gen: 0 as _ });
        }
        let prim = match self.r.resolve(old) {
            Ok(Primitive::Null) | Err(_) => return Primitive::Null,
            Ok(p) => p,
        };
        // Never drag in other pages, the old page tree or the old catalog.
        let blocked = match &prim {
            Primitive::Dictionary(d) => type_is(d, &["Page", "Pages", "Catalog"]),
            Primitive::Stream(s) => type_is(&s.info, &["Page", "Pages", "Catalog"]),
            _ => false,
        };
        if blocked {
            return Primitive::Null;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.map.insert(key, id);
        self.queue.push_back((id, prim));
        Primitive::Reference(PlainRef { id: id as _, gen: 0 as _ })
    }

    fn remap_dict(&mut self, d: &Dictionary) -> Dictionary {
        let mut out = Dictionary::new();
        for (k, v) in d.iter() {
            out.insert(k.clone(), self.remap(v));
        }
        out
    }

    fn remap(&mut self, p: &Primitive) -> Primitive {
        match p {
            Primitive::Reference(r) => self.map_ref(*r),
            Primitive::Array(a) => Primitive::Array(a.iter().map(|x| self.remap(x)).collect()),
            Primitive::Dictionary(d) => Primitive::Dictionary(self.remap_dict(d)),
            other => other.clone(),
        }
    }
}

fn extract_impl(src: &Path, dst: &Path, n: u32, password: &[u8]) -> Res<u32> {
    let mmap = map_file(src)?;
    let file = FileOptions::uncached().password(password).load(mmap).map_err(e)?;
    let r = file.resolver();

    // Locate the page tree via the raw catalog dictionary.
    let root_ref = file.trailer.root.get_ref().get_inner();
    let root = as_dict(r.resolve(root_ref).map_err(e)?).ok_or("catalog is not a dictionary")?;
    let pages_ref = match root.get("Pages") {
        Some(Primitive::Reference(p)) => *p,
        _ => return Err("catalog has no /Pages".into()),
    };

    let mut selected = Vec::new();
    collect_pages(&r, pages_ref, &Dictionary::new(), &mut selected, n as usize, &mut HashSet::new(), 0)?;
    if selected.is_empty() {
        return Err("no pages found".into());
    }
    let k = selected.len() as u32;

    // ids: 1 = catalog, 2 = pages node, 3..3+k = pages, then BFS descendants.
    let mut c = Copier { r: &r, map: HashMap::new(), queue: VecDeque::new(), next_id: 3 + k };
    for (i, (old, dict)) in selected.into_iter().enumerate() {
        let id = 3 + i as u32;
        c.map.insert((old.id as u64, old.gen as u64), id);
        c.queue.push_back((id, Primitive::Dictionary(dict)));
    }

    let out_file = File::create(dst).map_err(|x| format!("cannot create {}: {x}", dst.display()))?;
    let mut w = Counter { inner: BufWriter::with_capacity(256 * 1024, out_file), pos: 0 };
    let mut offsets: Vec<u64> = vec![0; 3 + k as usize];

    w.write_all(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n").map_err(e)?;

    offsets[1] = w.pos;
    w.write_all(b"1 0 obj\n<</Type/Catalog/Pages 2 0 R>>\nendobj\n").map_err(e)?;

    offsets[2] = w.pos;
    write!(w, "2 0 obj\n<</Type/Pages/Count {k}/Kids[").map_err(e)?;
    for i in 0..k {
        write!(w, "{} 0 R ", 3 + i).map_err(e)?;
    }
    w.write_all(b"]>>\nendobj\n").map_err(e)?;

    // FIFO processing == ascending id order, so offsets[] fills in order.
    while let Some((id, prim)) = c.queue.pop_front() {
        let pos = w.pos;
        if offsets.len() <= id as usize {
            offsets.resize(id as usize + 1, 0);
        }
        offsets[id as usize] = pos;
        write!(w, "{id} 0 obj\n").map_err(e)?;
        match prim {
            Primitive::Stream(s) => {
                let data = s.raw_data(&r).map_err(e)?; // one stream at a time
                let mut info = c.remap_dict(&s.info);
                let len = i32::try_from(data.len()).map_err(|_| "stream too large")?;
                info.insert("Length", Primitive::Integer(len));
                write_dict(&info, &mut w).map_err(e)?;
                w.write_all(b"\nstream\n").map_err(e)?;
                w.write_all(&data).map_err(e)?;
                w.write_all(b"\nendstream").map_err(e)?;
            }
            other => {
                let mut p = c.remap(&other);
                if id >= 3 && id < 3 + k {
                    if let Primitive::Dictionary(d) = &mut p {
                        d.insert("Parent", Primitive::Reference(PlainRef { id: 2 as _, gen: 0 as _ }));
                        d.insert("Type", Primitive::Name("Page".into()));
                    }
                }
                write_prim(&p, &mut w).map_err(e)?;
            }
        }
        w.write_all(b"\nendobj\n").map_err(e)?;
    }

    let size = offsets.len();
    let xref_pos = w.pos;
    write!(w, "xref\n0 {size}\n0000000000 65535 f \n").map_err(e)?;
    for off in &offsets[1..] {
        write!(w, "{off:010} 00000 n \n").map_err(e)?;
    }
    write!(w, "trailer\n<</Size {size}/Root 1 0 R>>\nstartxref\n{xref_pos}\n%%EOF\n").map_err(e)?;
    w.flush().map_err(e)?;
    Ok(k)
}

// ---------------------------------------------------------------- Python API

fn guarded<T>(f: impl FnOnce() -> Res<T>) -> PyResult<T> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(msg)) => Err(PdfFastError::new_err(msg)),
        Err(_) => Err(PdfFastError::new_err("malformed PDF (parser panic)")),
    }
}

/// Return the number of pages in the PDF at `path`.
#[pyfunction]
#[pyo3(signature = (path, password=None))]
fn page_count(py: Python<'_>, path: PathBuf, password: Option<Vec<u8>>) -> PyResult<u32> {
    let pw = password.unwrap_or_default();
    py.detach(move || guarded(|| page_count_impl(&path, &pw)))
}

/// Write the first `n` pages of `src` to `dst`; returns pages written.
#[pyfunction]
#[pyo3(signature = (src, dst, n, password=None))]
fn extract_first_pages(
    py: Python<'_>,
    src: PathBuf,
    dst: PathBuf,
    n: u32,
    password: Option<Vec<u8>>,
) -> PyResult<u32> {
    if n == 0 {
        return Err(PyValueError::new_err("n must be >= 1"));
    }
    let pw = password.unwrap_or_default();
    py.detach(move || {
        let res = guarded(|| extract_impl(&src, &dst, n, &pw));
        if res.is_err() {
            let _ = std::fs::remove_file(&dst); // don't leave partial output
        }
        res
    })
}

#[pymodule]
fn pdf_fast(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(page_count, m)?)?;
    m.add_function(wrap_pyfunction!(extract_first_pages, m)?)?;
    m.add("PdfFastError", m.py().get_type::<PdfFastError>())?;
    Ok(())
}
