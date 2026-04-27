use std::{
    cell::RefCell,
    cmp::Ordering,
    collections::{HashMap, HashSet, VecDeque},
    fs,
    io::{self, ErrorKind, Read as _, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};

use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use chrono::{DateTime, Local};
use clap::Parser;

const ONE_PIXEL_BLACK: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIW2NgYGD4DwABBAEAwS2OUAAAAABJRU5ErkJggg==";
const SIX_MONTHS: i64 = (365 * 86400) / 2;
const MAX_IMAGE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Parser, Debug)]
#[command(
    name = "thmbs",
    disable_help_flag = true,
    about = "ls(1) listing supplemented with image thumbnails and dimensions"
)]
struct Cli {
    /// Print help
    #[arg(long, action = clap::ArgAction::HelpLong)]
    help: Option<bool>,

    /// Thumbnail width in terminal cells
    #[arg(long, default_value_t = 3)]
    width: u32,
    /// Thumbnail height in terminal cells
    #[arg(long, default_value_t = 1)]
    height: u32,

    /// Preserve image aspect ratio when rendering thumbnails
    #[arg(long = "preserve-ratio", alias = "preserve_ratio",
          default_value_t = true,
          num_args = 0..=1, default_missing_value = "true",
          action = clap::ArgAction::Set)]
    preserve_ratio: bool,
    /// Stretch thumbnails to fill width/height
    #[arg(long = "no-preserve-ratio", alias = "nopreserve_ratio",
          default_value_t = false, action = clap::ArgAction::SetTrue)]
    no_preserve_ratio: bool,

    /// Show image dimensions (WxH) next to each file
    #[arg(long, default_value_t = false, action = clap::ArgAction::SetTrue,
          overrides_with = "no_dimensions")]
    dimensions: bool,
    /// Hide image dimensions column
    #[arg(long = "no-dimensions", alias = "nodimensions",
          default_value_t = false, action = clap::ArgAction::SetTrue,
          overrides_with = "dimensions")]
    no_dimensions: bool,

    /// Include files whose dimensions cannot be determined
    #[arg(long, default_value_t = false, action = clap::ArgAction::SetTrue,
          overrides_with = "no_unknown")]
    unknown: bool,
    /// Skip files whose dimensions cannot be determined
    #[arg(long = "no-unknown", alias = "nounknown",
          default_value_t = false, action = clap::ArgAction::SetTrue,
          overrides_with = "unknown")]
    no_unknown: bool,

    // ls options
    /// List all entries except . and ..
    #[arg(short = 'A', action = clap::ArgAction::SetTrue)]
    a_upper: bool,
    /// Append type indicator (*/=@|) to entries
    #[arg(short = 'F', action = clap::ArgAction::SetTrue)]
    f_upper: bool,
    /// Recurse into subdirectories
    #[arg(short = 'R', action = clap::ArgAction::SetTrue)]
    r_upper: bool,
    /// With -l, show complete time information
    #[arg(short = 'T', action = clap::ArgAction::SetTrue)]
    t_upper: bool,
    /// List all entries including those starting with .
    #[arg(short = 'a', action = clap::ArgAction::SetTrue)]
    a: bool,
    /// List directories themselves, not their contents
    #[arg(short = 'd', action = clap::ArgAction::SetTrue)]
    d: bool,
    /// With -l, print sizes in human-readable format
    #[arg(short = 'h', action = clap::ArgAction::SetTrue)]
    h: bool,
    /// Print each file's inode number
    #[arg(short = 'i', action = clap::ArgAction::SetTrue)]
    i: bool,
    /// Display block counts in 1024-byte units
    #[arg(short = 'k', action = clap::ArgAction::SetTrue)]
    k: bool,
    /// Use long listing format
    #[arg(short = 'l', action = clap::ArgAction::SetTrue)]
    l: bool,
    /// Like -l, but show numeric UID/GID
    #[arg(short = 'n', action = clap::ArgAction::SetTrue)]
    n: bool,
    /// Like -l, but omit group information
    #[arg(short = 'o', action = clap::ArgAction::SetTrue)]
    o: bool,
    /// Append / to directory names
    #[arg(short = 'p', action = clap::ArgAction::SetTrue)]
    p: bool,
    /// Reverse sort order
    #[arg(short = 'r', action = clap::ArgAction::SetTrue)]
    r: bool,
    /// Print allocated block count for each file
    #[arg(short = 's', action = clap::ArgAction::SetTrue)]
    s: bool,
    /// Sort by modification time, newest first
    #[arg(short = 't', action = clap::ArgAction::SetTrue,
          overrides_with = "s_upper")]
    t: bool,
    /// Sort by file size, largest first
    #[arg(short = 'S', action = clap::ArgAction::SetTrue,
          overrides_with = "t")]
    s_upper: bool,
    /// Sort by/use access time instead of modification time
    #[arg(short = 'u', action = clap::ArgAction::SetTrue)]
    u: bool,
    /// Use same sort order for ties as the primary sort
    #[arg(short = 'y', action = clap::ArgAction::SetTrue)]
    y: bool,
    /// Sort by/use status change time (ctime)
    #[arg(short = 'c', action = clap::ArgAction::SetTrue)]
    c: bool,
    /// Alias for -c
    #[arg(short = 'U', action = clap::ArgAction::SetTrue)]
    u_upper: bool,
    /// strftime format for -l timestamps
    #[arg(short = 'D')]
    d_fmt: Option<String>,

    /// Files or directories to list (default: current directory)
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
        // With clap `overrides_with`, only one of each pair can be true at a time.
        let dimensions = c.dimensions;
        let unknown = c.unknown;
        // Perl side-effect: setting either dimensions or unknown sets the other.
        let (dimensions, unknown) = if c.dimensions || c.unknown {
            (true, true)
        } else if c.no_dimensions || c.no_unknown {
            (false, false)
        } else {
            (dimensions, unknown)
        };

        let preserve_ratio = if c.no_preserve_ratio {
            false
        } else {
            c.preserve_ratio
        };

        let t = c.t;
        let s_upper = c.s_upper;

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

/// Write bytes to `out`, replacing control bytes (< 0x20), DEL (0x7f),
/// and ESC (0x1b) with `?`. Matches the spirit of `ls -q`.
fn write_sanitized<W: Write>(out: &mut W, bytes: &[u8]) -> io::Result<()> {
    let mut buf = [0u8; 256];
    let mut n = 0;
    for &b in bytes {
        let ch = if b < 0x20 || b == 0x7f || b == 0x1b {
            b'?'
        } else {
            b
        };
        buf[n] = ch;
        n += 1;
        if n == buf.len() {
            out.write_all(&buf[..n])?;
            n = 0;
        }
    }
    if n > 0 {
        out.write_all(&buf[..n])?;
    }
    Ok(())
}

fn read_capped(file: &Path) -> io::Result<Vec<u8>> {
    let f = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(file)?;
    let md = f.metadata()?;
    if !md.is_file() {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    if md.len() > MAX_IMAGE_BYTES {
        return Err(io::Error::new(ErrorKind::InvalidInput, "file too large"));
    }
    let mut buf = Vec::with_capacity(md.len() as usize);
    f.take(MAX_IMAGE_BYTES).read_to_end(&mut buf)?;
    Ok(buf)
}

fn write_image<W: Write>(out: &mut W, file: &Path, size: u64, opts: &Opts) -> io::Result<()> {
    let display_name = file.as_os_str().as_bytes();
    let mut name_b64 = B64.encode(display_name);
    let mut size_str = size.to_string();

    let encoded = if size > MAX_IMAGE_BYTES {
        name_b64 = B64.encode(b"one_pixel_black");
        size_str = ONE_PIXEL_BLACK.len().to_string();
        ONE_PIXEL_BLACK.to_string()
    } else {
        match read_capped(file) {
            Ok(bytes) if !bytes.is_empty() => B64.encode(&bytes),
            _ => {
                name_b64 = B64.encode(b"one_pixel_black");
                size_str = ONE_PIXEL_BLACK.len().to_string();
                ONE_PIXEL_BLACK.to_string()
            }
        }
    };

    write!(
        out,
        "\x1b]1337;File=name={};size={};inline=1;height={};width={};preserveAspectRatio={}:{}\x07",
        name_b64,
        size_str,
        opts.img_height,
        opts.img_width,
        if opts.preserve_ratio { "true" } else { "false" },
        encoded
    )
}

fn write_placeholder<W: Write>(out: &mut W, opts: &Opts) -> io::Result<()> {
    let name_b64 = B64.encode(b"one_pixel_black");
    write!(
        out,
        "\x1b]1337;File=name={};size={};inline=1;height={};width={};preserveAspectRatio={}:{}\x07",
        name_b64,
        ONE_PIXEL_BLACK.len(),
        opts.img_height,
        opts.img_width,
        if opts.preserve_ratio { "true" } else { "false" },
        ONE_PIXEL_BLACK
    )
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

fn ls_sort(paths: &mut [PathBuf], opts: &Opts) {
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
    uzers::get_user_by_uid(uid).and_then(|u| u.name().to_str().map(String::from))
}

fn gid_name(gid: u32) -> Option<String> {
    uzers::get_group_by_gid(gid).and_then(|g| g.name().to_str().map(String::from))
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

/// Returns the bytes used as the displayed filename for `path`.
/// When `parent` is set, returns just the file name; otherwise the full path.
fn entry_filename_bytes(path: &Path, parent: Option<&Path>) -> Vec<u8> {
    if parent.is_some() {
        path.file_name()
            .map(|s| s.as_bytes().to_vec())
            .unwrap_or_default()
    } else {
        path.as_os_str().as_bytes().to_vec()
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

fn read_dir_split(path: &Path, opts: &Opts) -> io::Result<(Vec<PathBuf>, Vec<PathBuf>)> {
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

fn do_ls<W: Write>(
    out: &mut W,
    parent: Option<&Path>,
    mut paths: Vec<PathBuf>,
    opts: &Opts,
) -> io::Result<()> {
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
            let name_bytes = entry_filename_bytes(&p, parent);
            let is_hidden_dotfile = name_bytes.starts_with(b".")
                && !name_bytes.starts_with(b"..")
                && name_bytes.as_slice() != b".";
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

    if (opts.show_blocks || opts.l) && parent.is_some() && !opts.d {
        let total = if opts.k {
            blocks_total / 2
        } else {
            blocks_total
        };
        writeln!(out, "total {}", total)?;
    }

    for e in &entries {
        let placeholder = !e.md.file_type().is_file()
            || e.md.len() == 0
            || (opts.dimensions && e.dims.is_none() && !opts.unknown);
        if placeholder {
            write_placeholder(out, opts)?;
        } else {
            write_image(out, &e.path, e.md.len(), opts)?;
        }

        if opts.dimensions && (w_dim_w > 0 || w_dim_h > 0) {
            let mw = w_dim_w.max(1);
            let mh = w_dim_h.max(1);
            if let Some((w, h)) = e.dims {
                write!(out, " [{:>mw$} x {:>mh$}] ", w, h, mw = mw, mh = mh)?;
            } else {
                write!(out, " {:>mw$}   {:>mh$}   ", "", "", mw = mw, mh = mh)?;
            }
        }

        if opts.i {
            write!(out, " {:>w$}", e.md.ino(), w = w_ino)?;
        }
        if opts.s {
            write!(out, " {:>w$}", e.md.blocks(), w = w_blocks)?;
        }
        if opts.l {
            write!(out, " {}", format_mode(e.md.mode()))?;
            write!(out, " {:>w$}", e.md.nlink(), w = w_nlink)?;
            let owner = if opts.n {
                e.md.uid().to_string()
            } else {
                uid_name(e.md.uid()).unwrap_or_else(|| e.md.uid().to_string())
            };
            write!(out, " {:>w$}", owner, w = w_owner)?;
            if !opts.o {
                let group = if opts.n {
                    e.md.gid().to_string()
                } else {
                    gid_name(e.md.gid()).unwrap_or_else(|| e.md.gid().to_string())
                };
                write!(out, "  {:>w$}", group, w = w_group)?;
            }
            if opts.h {
                write!(out, "  {:>4}", format_human(e.md.len()))?;
            } else {
                write!(out, "  {:>w$}", e.md.len(), w = w_bytes)?;
            }
            let t = if opts.c { e.md.ctime() } else { e.md.mtime() };
            write!(out, " {}", format_time(t, opts))?;
        }

        let name = entry_filename_bytes(&e.path, parent);
        out.write_all(b" ")?;
        write_sanitized(out, &name)?;
        let suffix = get_f_type(&e.md, opts);
        write!(out, "{}", suffix)?;
        writeln!(out)?;
    }

    // Bound the stat cache: clear at end of each directory listing.
    STAT_CACHE.with(|c| c.borrow_mut().clear());

    Ok(())
}

fn run() -> io::Result<()> {
    let cli = Cli::parse();
    let (opts, mut paths) = Opts::from_cli(cli);

    let stdout = io::stdout();
    let mut out = stdout.lock();

    let mut do_header = paths.len() > 1;
    if paths.is_empty() {
        paths.push(PathBuf::from("."));
    }

    let mut files: Vec<PathBuf> = Vec::new();
    let mut dirs: VecDeque<PathBuf> = VecDeque::new();
    for p in paths {
        match lstat(&p) {
            None => {
                writeln!(
                    io::stderr(),
                    "thmbs: {}: No such file or directory",
                    p.display()
                )?;
            }
            Some(md) if md.file_type().is_file() || (opts.d && !md.file_type().is_dir()) => {
                files.push(p);
            }
            Some(md) if md.file_type().is_dir() => {
                if opts.d {
                    files.push(p);
                } else {
                    dirs.push_back(p);
                }
            }
            Some(_) => files.push(p),
        }
    }

    ls_sort(&mut files, &opts);
    {
        let mut tmp: Vec<PathBuf> = dirs.drain(..).collect();
        ls_sort(&mut tmp, &opts);
        dirs = VecDeque::from(tmp);
    }

    if !files.is_empty() {
        do_ls(&mut out, None, files, &opts)?;
    }

    // Symlink-loop guard for -R: track visited (dev, ino).
    let mut visited: HashSet<(u64, u64)> = HashSet::new();
    // Seed visited with initially-listed dirs so we never re-enter them.
    for d in &dirs {
        if let Some(md) = lstat(d) {
            visited.insert((md.dev(), md.ino()));
        }
    }

    let mut do_newline = false;
    while let Some(path) = dirs.pop_front() {
        let (f, d) = match read_dir_split(&path, &opts) {
            Ok(t) => t,
            Err(e) => {
                writeln!(
                    io::stderr(),
                    "Unable to open directory {}: {}",
                    path.display(),
                    e
                )?;
                continue;
            }
        };
        if do_newline {
            writeln!(out)?;
        }
        if do_header {
            write!(out, "")?;
            // Sanitize the directory header path bytes.
            let header = path.as_os_str().as_bytes();
            write_sanitized(&mut out, header)?;
            writeln!(out, ":")?;
        }
        do_header = true;

        let mut combined = f;
        combined.extend(d.iter().cloned());
        do_ls(&mut out, Some(&path), combined, &opts)?;

        if opts.r_upper {
            for sub in d {
                let name = sub
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if name == "." || name == ".." {
                    continue;
                }
                if let Some(md) = lstat(&sub) {
                    let key = (md.dev(), md.ino());
                    if visited.insert(key) {
                        dirs.push_back(sub);
                    }
                }
            }
        }
        do_newline = true;
    }

    out.flush()?;
    Ok(())
}

fn main() {
    match run() {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::BrokenPipe => {
            std::process::exit(0);
        }
        Err(e) => {
            let _ = writeln!(io::stderr(), "thmbs: {}", e);
            std::process::exit(1);
        }
    }
}
