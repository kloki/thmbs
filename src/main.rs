use std::{
    cell::RefCell,
    cmp::Ordering,
    collections::{HashMap, HashSet},
    ffi::CStr,
    fs,
    io::Write,
    os::unix::{
        ffi::OsStrExt,
        fs::{FileTypeExt, MetadataExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};

use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use chrono::{DateTime, Local};
use clap::Parser;

const ONE_PIXEL_BLACK: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIW2NgYGD4DwABBAEAwS2OUAAAAABJRU5ErkJggg==";
const SIX_MONTHS: i64 = (365 * 86400) / 2;

#[derive(Parser, Debug)]
#[command(
    name = "ill",
    disable_help_flag = true,
    about = "ls(1) listing supplemented with image thumbnails and dimensions"
)]
struct Cli {
    #[arg(long, default_value_t = 3)]
    width: u32,
    #[arg(long, default_value_t = 1)]
    height: u32,

    #[arg(long = "preserve-ratio", alias = "preserve_ratio",
          default_value_t = true,
          num_args = 0..=1, default_missing_value = "true",
          action = clap::ArgAction::Set)]
    preserve_ratio: bool,
    #[arg(long = "no-preserve-ratio", alias = "nopreserve_ratio",
          default_value_t = false, action = clap::ArgAction::SetTrue)]
    no_preserve_ratio: bool,

    #[arg(long, default_value_t = false, action = clap::ArgAction::SetTrue)]
    dimensions: bool,
    #[arg(long = "no-dimensions", alias = "nodimensions",
          default_value_t = false, action = clap::ArgAction::SetTrue)]
    no_dimensions: bool,

    #[arg(long, default_value_t = false, action = clap::ArgAction::SetTrue)]
    unknown: bool,
    #[arg(long = "no-unknown", alias = "nounknown",
          default_value_t = false, action = clap::ArgAction::SetTrue)]
    no_unknown: bool,

    #[arg(long)]
    method: Option<String>,

    // ls options
    #[arg(short = 'A', action = clap::ArgAction::SetTrue)]
    a_upper: bool,
    #[arg(short = 'F', action = clap::ArgAction::SetTrue)]
    f_upper: bool,
    #[arg(short = 'R', action = clap::ArgAction::SetTrue)]
    r_upper: bool,
    #[arg(short = 'T', action = clap::ArgAction::SetTrue)]
    t_upper: bool,
    #[arg(short = 'a', action = clap::ArgAction::SetTrue)]
    a: bool,
    #[arg(short = 'd', action = clap::ArgAction::SetTrue)]
    d: bool,
    #[arg(short = 'h', action = clap::ArgAction::SetTrue)]
    h: bool,
    #[arg(short = 'i', action = clap::ArgAction::SetTrue)]
    i: bool,
    #[arg(short = 'k', action = clap::ArgAction::SetTrue)]
    k: bool,
    #[arg(short = 'l', action = clap::ArgAction::SetTrue)]
    l: bool,
    #[arg(short = 'n', action = clap::ArgAction::SetTrue)]
    n: bool,
    #[arg(short = 'o', action = clap::ArgAction::SetTrue)]
    o: bool,
    #[arg(short = 'p', action = clap::ArgAction::SetTrue)]
    p: bool,
    #[arg(short = 'r', action = clap::ArgAction::SetTrue)]
    r: bool,
    #[arg(short = 's', action = clap::ArgAction::SetTrue)]
    s: bool,
    #[arg(short = 't', action = clap::ArgAction::SetTrue)]
    t: bool,
    #[arg(short = 'S', action = clap::ArgAction::SetTrue)]
    s_upper: bool,
    #[arg(short = 'u', action = clap::ArgAction::SetTrue)]
    u: bool,
    #[arg(short = 'y', action = clap::ArgAction::SetTrue)]
    y: bool,
    #[arg(short = 'c', action = clap::ArgAction::SetTrue)]
    c: bool,
    #[arg(short = 'U', action = clap::ArgAction::SetTrue)]
    u_upper: bool,
    #[arg(short = 'D')]
    d_fmt: Option<String>,

    paths: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
struct Opts {
    img_width: u32,
    img_height: u32,
    preserve_ratio: bool,

    dimensions: bool,
    unknown: bool,

    a_upper: bool,
    f_upper: bool,
    r_upper: bool,
    t_upper: bool,
    a: bool,
    d: bool,
    h: bool,
    i: bool,
    k: bool,
    l: bool,
    n: bool,
    o: bool,
    p: bool,
    r: bool,
    s: bool,
    t: bool,
    s_upper: bool,
    u: bool,
    y: bool,
    c: bool,
    d_fmt: Option<String>,
    show_blocks: bool,
}

impl Opts {
    fn from_cli(c: Cli) -> (Self, Vec<PathBuf>) {
        // dimensions/no_dimensions: --no-X wins if set; otherwise --X.
        // Per Perl: each toggles 'unknown' too.
        let mut dimensions = c.dimensions;
        if c.no_dimensions {
            dimensions = false;
        }
        let mut unknown = c.unknown;
        if c.no_unknown {
            unknown = false;
        }
        // Perl side-effect: setting either dimensions or unknown sets the other.
        if c.dimensions || c.no_dimensions {
            unknown = !c.no_dimensions || c.unknown;
        }
        if c.unknown || c.no_unknown {
            dimensions = !c.no_unknown || c.dimensions;
        }

        let preserve_ratio = if c.no_preserve_ratio {
            false
        } else {
            c.preserve_ratio
        };

        let t = c.t;
        let mut s_upper = c.s_upper;
        // Perl: -t deletes -S, -S deletes -t. Last one wins via clap; both true => prefer -t (unspecified, match Perl: each clears the other when seen — final state depends on order, which clap doesn't track; pick: if both set, keep -t).
        if t && s_upper {
            s_upper = false;
        }

        let mut r_upper = c.r_upper;
        if c.d {
            r_upper = false;
        }

        let mut t_upper = c.t_upper;
        if c.d_fmt.is_some() {
            t_upper = false;
        }

        let mut l = c.l;
        if c.n || c.o {
            l = true;
        }

        let show_blocks = c.s;

        // -c implies -U (per Perl alias 'c|U'); we collapse to c
        let c_flag = c.c || c.u_upper;

        let opts = Opts {
            img_width: c.width,
            img_height: c.height,
            preserve_ratio,
            dimensions,
            unknown,
            a_upper: c.a_upper,
            f_upper: c.f_upper,
            r_upper,
            t_upper,
            a: c.a,
            d: c.d,
            h: c.h,
            i: c.i,
            k: c.k,
            l,
            n: c.n,
            o: c.o,
            p: c.p,
            r: c.r,
            s: c.s,
            t,
            s_upper,
            u: c.u,
            y: c.y,
            c: c_flag,
            d_fmt: c.d_fmt,
            show_blocks,
        };
        (opts, c.paths)
    }
}

thread_local! {
    static STAT_CACHE: RefCell<HashMap<PathBuf, Option<fs::Metadata>>> = RefCell::new(HashMap::new());
    static FAILED_TYPES: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

fn lstat(path: &Path) -> Option<fs::Metadata> {
    STAT_CACHE.with(|c| {
        if let Some(v) = c.borrow().get(path) {
            return v.clone();
        }
        let m = fs::symlink_metadata(path).ok();
        c.borrow_mut().insert(path.to_path_buf(), m.clone());
        m
    })
}

fn write_image<W: Write>(out: &mut W, file: &Path, size: u64, opts: &Opts) {
    let display_name = file.as_os_str().as_bytes();
    let mut name_b64 = B64.encode(display_name);
    let mut size_str = size.to_string();

    let encoded = match fs::read(file) {
        Ok(bytes) if !bytes.is_empty() => B64.encode(&bytes),
        _ => {
            name_b64 = B64.encode(b"one_pixel_black");
            size_str = ONE_PIXEL_BLACK.len().to_string();
            ONE_PIXEL_BLACK.to_string()
        }
    };

    let _ = write!(
        out,
        "\x1b]1337;File=name={};size={};inline=1;height={};width={};preserveAspectRatio={}:{}\x07",
        name_b64,
        size_str,
        opts.img_height,
        opts.img_width,
        if opts.preserve_ratio { "true" } else { "false" },
        encoded
    );
}

fn write_placeholder<W: Write>(out: &mut W, opts: &Opts) {
    let name_b64 = B64.encode(b"one_pixel_black");
    let _ = write!(
        out,
        "\x1b]1337;File=name={};size={};inline=1;height={};width={};preserveAspectRatio={}:{}\x07",
        name_b64,
        ONE_PIXEL_BLACK.len(),
        opts.img_height,
        opts.img_width,
        if opts.preserve_ratio { "true" } else { "false" },
        ONE_PIXEL_BLACK
    );
}

fn get_dimensions(path: &Path) -> Option<(usize, usize)> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase());
    if let Some(ref e) = ext {
        let skip = FAILED_TYPES.with(|f| f.borrow().contains(e));
        if skip {
            return None;
        }
    }
    match imagesize::size(path) {
        Ok(d) => Some((d.width, d.height)),
        Err(_) => {
            if let Some(e) = ext {
                FAILED_TYPES.with(|f| {
                    f.borrow_mut().insert(e);
                });
            }
            None
        }
    }
}

fn ls_sort(paths: &mut Vec<PathBuf>, opts: &Opts) {
    let samesort = opts.y || std::env::var("LS_SAMESORT").is_ok();
    if opts.t {
        paths.sort_by(|a, b| {
            let am = lstat(a).map(|m| m.mtime()).unwrap_or(0);
            let bm = lstat(b).map(|m| m.mtime()).unwrap_or(0);
            match bm.cmp(&am) {
                Ordering::Equal => {
                    if samesort {
                        b.cmp(a)
                    } else {
                        a.cmp(b)
                    }
                }
                o => o,
            }
        });
    } else if opts.c {
        paths.sort();
    } else if opts.u {
        paths.sort_by(|a, b| {
            let am = lstat(a).map(|m| m.atime()).unwrap_or(0);
            let bm = lstat(b).map(|m| m.atime()).unwrap_or(0);
            am.cmp(&bm)
        });
    } else if opts.s_upper {
        paths.sort_by(|a, b| {
            let asz = lstat(a).map(|m| m.size()).unwrap_or(0);
            let bsz = lstat(b).map(|m| m.size()).unwrap_or(0);
            match bsz.cmp(&asz) {
                Ordering::Equal => a.cmp(b),
                o => o,
            }
        });
    } else {
        paths.sort();
    }
    if opts.r {
        paths.reverse();
    }
}

fn format_mode(mode: u32) -> String {
    let perm_chars = ["---", "--x", "-w-", "-wx", "r--", "r-x", "rw-", "rwx"];
    let ftype_chars = [
        ".", "p", "c", "?", "d", "?", "b", "?", "-", "?", "l", "?", "s", "?", "?", "?",
    ];
    let ft_idx = ((mode & 0o170000) >> 12) as usize;
    let ftype = if ft_idx == 0 { "" } else { ftype_chars[ft_idx] };

    let setids = (mode & 0o7000) >> 9;
    let mut p = [
        perm_chars[((mode & 0o700) >> 6) as usize].to_string(),
        perm_chars[((mode & 0o070) >> 3) as usize].to_string(),
        perm_chars[(mode & 0o007) as usize].to_string(),
    ];
    if setids & 0o1 != 0 {
        let last = p[2].pop().unwrap();
        p[2].push(if last == 'x' { 't' } else { 'T' });
    }
    if setids & 0o4 != 0 {
        let last = p[0].pop().unwrap();
        p[0].push(if last == 'x' { 's' } else { 'S' });
    }
    if setids & 0o2 != 0 {
        let last = p[1].pop().unwrap();
        p[1].push(if last == 'x' { 's' } else { 'S' });
    }
    format!("{}{}{}{}", ftype, p[0], p[1], p[2])
}

fn format_human(bytes: u64) -> String {
    let units = ["B", "K", "M", "G", "T", "P"];
    let s = bytes.to_string();
    let scale = (s.len().saturating_sub(1)) / 3;
    let scale = scale.min(units.len() - 1);
    let float = bytes as f64 / 1024_f64.powi(scale as i32);
    let int_part = float.trunc() as u64;
    let int_len = int_part.to_string().len();
    if s.len() < 3 || int_len >= 2 {
        let frac = float - int_part as f64;
        let rounded = if frac < 0.5 { int_part } else { int_part + 1 };
        format!("{}{}", rounded, units[scale])
    } else {
        format!("{:.1}{}", float, units[scale])
    }
}

fn format_time(time: i64, opts: &Opts) -> String {
    let dt: DateTime<Local> = DateTime::from_timestamp(time, 0)
        .map(|t| t.with_timezone(&Local))
        .unwrap_or_else(Local::now);
    let now = Local::now().timestamp();
    let fmt: &str = if let Some(f) = &opts.d_fmt {
        f.as_str()
    } else if opts.t_upper {
        "%b %e %H:%M:%S %Y"
    } else if time + SIX_MONTHS > now && time < now + SIX_MONTHS {
        "%b %e %H:%M"
    } else {
        "%b %e  %Y"
    };
    dt.format(fmt).to_string()
}

fn uid_name(uid: u32) -> Option<String> {
    unsafe {
        let pw = libc::getpwuid(uid as libc::uid_t);
        if pw.is_null() {
            return None;
        }
        let n = (*pw).pw_name;
        if n.is_null() {
            return None;
        }
        CStr::from_ptr(n).to_str().ok().map(|s| s.to_string())
    }
}

fn gid_name(gid: u32) -> Option<String> {
    unsafe {
        let gr = libc::getgrgid(gid as libc::gid_t);
        if gr.is_null() {
            return None;
        }
        let n = (*gr).gr_name;
        if n.is_null() {
            return None;
        }
        CStr::from_ptr(n).to_str().ok().map(|s| s.to_string())
    }
}

fn get_f_type(md: &fs::Metadata, opts: &Opts) -> &'static str {
    let ft = md.file_type();
    if ft.is_dir() && (opts.p || opts.f_upper) {
        return "/";
    }
    if !opts.f_upper {
        return "";
    }
    if ft.is_symlink() {
        return "@";
    }
    if ft.is_fifo() {
        return "|";
    }
    if ft.is_socket() {
        return "=";
    }
    if md.permissions().mode() & 0o111 != 0 {
        return "*";
    }
    ""
}

struct Entry {
    path: PathBuf,
    md: fs::Metadata,
    dims: Option<(usize, usize)>,
}

fn entry_filename(path: &Path, parent: Option<&Path>) -> String {
    if parent.is_some() {
        path.file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        path.to_string_lossy().into_owned()
    }
}

fn dot_filtered(name: &str, opts: &Opts) -> bool {
    if opts.a {
        return false;
    }
    if opts.a_upper {
        return name == "." || name == "..";
    }
    name.starts_with('.')
}

fn read_dir_split(path: &Path, opts: &Opts) -> std::io::Result<(Vec<PathBuf>, Vec<PathBuf>)> {
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if dot_filtered(&name_str, opts) {
            continue;
        }
        let p = entry.path();
        let md = lstat(&p);
        if let Some(m) = md {
            if m.file_type().is_dir() {
                dirs.push(p);
            } else {
                files.push(p);
            }
        } else {
            files.push(p);
        }
    }
    Ok((files, dirs))
}

fn do_ls<W: Write>(out: &mut W, parent: Option<&Path>, mut paths: Vec<PathBuf>, opts: &Opts) {
    ls_sort(&mut paths, opts);

    let mut entries: Vec<Entry> = Vec::with_capacity(paths.len());
    let mut blocks_total: u64 = 0;

    for p in paths.into_iter() {
        let md = match lstat(&p) {
            Some(m) => m,
            None => continue,
        };
        let dims = if opts.dimensions && md.file_type().is_file() && md.len() > 0 {
            get_dimensions(&p)
        } else {
            None
        };

        if opts.show_blocks || opts.l {
            let name = entry_filename(&p, parent);
            let is_hidden_dotfile = name.starts_with('.') && !name.starts_with("..") && name != ".";
            // Perl: if not -d and ($opts{a} or filename !~ /^\.[^.]+/)
            if !md.file_type().is_dir() && (opts.a || !is_hidden_dotfile) {
                blocks_total += md.blocks();
            }
        }
        entries.push(Entry { path: p, md, dims });
    }

    // column widths
    let mut w_dim_w = 0usize;
    let mut w_dim_h = 0usize;
    let mut w_blocks = 0usize;
    let mut w_ino = 0usize;
    let mut w_bytes = 0usize;
    let mut w_bytesh = 0usize;
    let mut w_owner = 0usize;
    let mut w_group = 0usize;
    let mut w_nlink = 0usize;

    for e in &entries {
        if let Some((w, h)) = e.dims {
            w_dim_w = w_dim_w.max(w.to_string().len());
            w_dim_h = w_dim_h.max(h.to_string().len());
        }
        if opts.show_blocks || opts.l {
            w_blocks = w_blocks.max(e.md.blocks().to_string().len());
            w_nlink = w_nlink.max(e.md.nlink().to_string().len());
        }
        if opts.i {
            w_ino = w_ino.max(e.md.ino().to_string().len());
        }
        if opts.l {
            w_bytes = w_bytes.max(e.md.len().to_string().len());
            if opts.h {
                w_bytesh = w_bytesh.max(format_human(e.md.len()).len());
            }
            let owner = if opts.n {
                e.md.uid().to_string()
            } else {
                uid_name(e.md.uid()).unwrap_or_else(|| e.md.uid().to_string())
            };
            w_owner = w_owner.max(owner.len());
            if !opts.o {
                let group = if opts.n {
                    e.md.gid().to_string()
                } else {
                    gid_name(e.md.gid()).unwrap_or_else(|| e.md.gid().to_string())
                };
                w_group = w_group.max(group.len());
            }
        }
    }

    if opts.show_blocks || opts.l {
        if let Some(p) = parent {
            if !opts.d {
                let total = if opts.k {
                    blocks_total / 2
                } else {
                    blocks_total
                };
                let _ = writeln!(out, "total {}", total);
                let _ = p; // suppress unused warning if needed
            }
        }
    }

    for e in &entries {
        let placeholder = !e.md.file_type().is_file()
            || e.md.len() == 0
            || (opts.dimensions && e.dims.is_none() && !opts.unknown);
        if placeholder {
            write_placeholder(out, opts);
        } else {
            write_image(out, &e.path, e.md.len(), opts);
        }

        if opts.dimensions && (w_dim_w > 0 || w_dim_h > 0) {
            let mw = w_dim_w.max(1);
            let mh = w_dim_h.max(1);
            if let Some((w, h)) = e.dims {
                let _ = write!(out, " [{:>mw$} x {:>mh$}] ", w, h, mw = mw, mh = mh);
            } else {
                let _ = write!(out, " {:>mw$}   {:>mh$}   ", "", "", mw = mw, mh = mh);
            }
        }

        if opts.i {
            let _ = write!(out, " {:>w$}", e.md.ino(), w = w_ino);
        }
        if opts.s {
            let _ = write!(out, " {:>w$}", e.md.blocks(), w = w_blocks);
        }
        if opts.l {
            let _ = write!(out, " {}", format_mode(e.md.mode()));
            let _ = write!(out, " {:>w$}", e.md.nlink(), w = w_nlink);
            let owner = if opts.n {
                e.md.uid().to_string()
            } else {
                uid_name(e.md.uid()).unwrap_or_else(|| e.md.uid().to_string())
            };
            let _ = write!(out, " {:>w$}", owner, w = w_owner);
            if !opts.o {
                let group = if opts.n {
                    e.md.gid().to_string()
                } else {
                    gid_name(e.md.gid()).unwrap_or_else(|| e.md.gid().to_string())
                };
                let _ = write!(out, "  {:>w$}", group, w = w_group);
            }
            if opts.h {
                let _ = write!(out, "  {:>4}", format_human(e.md.len()));
            } else {
                let _ = write!(out, "  {:>w$}", e.md.len(), w = w_bytes);
            }
            let t = if opts.c { e.md.ctime() } else { e.md.mtime() };
            let _ = write!(out, " {}", format_time(t, opts));
        }

        let name = entry_filename(&e.path, parent);
        let _ = write!(out, " {}", name);
        let suffix = get_f_type(&e.md, opts);
        let _ = write!(out, "{}", suffix);
        let _ = writeln!(out);
    }
    let _ = w_bytesh;
}

fn main() {
    let cli = Cli::parse();
    let (opts, mut paths) = Opts::from_cli(cli);

    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    let mut do_header = paths.len() > 1;
    if paths.is_empty() {
        paths.push(PathBuf::from("."));
    }

    let mut files: Vec<PathBuf> = Vec::new();
    let mut dirs: Vec<PathBuf> = Vec::new();
    for p in paths {
        match lstat(&p) {
            None => {
                let _ = writeln!(
                    std::io::stderr(),
                    "ill: {}: No such file or directory",
                    p.display()
                );
            }
            Some(md) if md.file_type().is_file() || (opts.d && !md.file_type().is_dir()) => {
                files.push(p);
            }
            Some(md) if md.file_type().is_dir() => {
                if opts.d {
                    files.push(p);
                } else {
                    dirs.push(p);
                }
            }
            Some(_) => files.push(p),
        }
    }

    ls_sort(&mut files, &opts);
    ls_sort(&mut dirs, &opts);

    if !files.is_empty() {
        do_ls(&mut out, None, files, &opts);
    }

    let mut do_newline = false;
    while !dirs.is_empty() {
        let path = dirs.remove(0);
        let (f, d) = match read_dir_split(&path, &opts) {
            Ok(t) => t,
            Err(e) => {
                let _ = writeln!(
                    std::io::stderr(),
                    "Unable to open directory {}: {}",
                    path.display(),
                    e
                );
                continue;
            }
        };
        if do_newline {
            let _ = writeln!(out);
        }
        if do_header {
            let _ = writeln!(out, "{}:", path.display());
        }
        do_header = true;

        let mut combined = f;
        combined.extend(d.iter().cloned());
        do_ls(&mut out, Some(&path), combined, &opts);

        if opts.r_upper {
            for sub in d {
                let name = sub
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if name == "." || name == ".." {
                    continue;
                }
                dirs.push(sub);
            }
        }
        do_newline = true;
    }
}
