#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::struct_excessive_bools,
    clippy::similar_names,
    clippy::doc_markdown,
    // Stylistic; not worth the churn across all write!/format! sites.
    clippy::uninlined_format_args,
    // Numeric literals in tests are clearer without separators here.
    clippy::unreadable_literal,
    // The single-match form mirrors adjacent multi-arm matches; keep symmetry.
    clippy::single_match_else
)]

use std::{
    cell::RefCell,
    cmp::Ordering,
    collections::{HashMap, HashSet, VecDeque},
    ffi::CStr,
    fs,
    io::{self, ErrorKind, Read as _, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};

use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use clap::Parser;
use jiff::{Timestamp, Zoned, tz::TimeZone};

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

impl Cli {
    fn show_dimensions(&self) -> bool {
        self.dimensions || self.unknown
    }

    fn show_unknown(&self) -> bool {
        self.unknown
    }

    fn preserve_ratio_resolved(&self) -> bool {
        if self.no_preserve_ratio {
            false
        } else {
            self.preserve_ratio
        }
    }

    fn sort_t(&self) -> bool {
        self.t
    }

    fn sort_s_upper(&self) -> bool {
        self.s_upper
    }

    fn recurse(&self) -> bool {
        if self.d {
            return false;
        }
        self.r_upper
    }

    fn t_upper_resolved(&self) -> bool {
        if self.d_fmt.is_some() {
            return false;
        }
        self.t_upper
    }

    fn long(&self) -> bool {
        self.l || self.n || self.o
    }

    fn show_blocks(&self) -> bool {
        self.s
    }

    fn ctime(&self) -> bool {
        // -c implies -U (per Perl alias 'c|U')
        self.c || self.u_upper
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

fn read_image_capped(file: &Path, size: u64) -> io::Result<Vec<u8>> {
    let f = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(file)?;
    let md = f.metadata()?;
    if !md.file_type().is_file() {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    if md.len() > MAX_IMAGE_BYTES {
        return Err(io::Error::new(ErrorKind::InvalidInput, "file too large"));
    }
    let cap = size.min(MAX_IMAGE_BYTES) as usize;
    let mut buf = Vec::with_capacity(cap);
    f.take(MAX_IMAGE_BYTES).read_to_end(&mut buf)?;
    Ok(buf)
}

fn write_image<W: Write>(out: &mut W, file: &Path, size: u64, cli: &Cli) {
    let display_name = file.as_os_str().as_bytes();
    let mut name_b64 = B64.encode(display_name);
    let mut size_str = size.to_string();

    let encoded = match read_image_capped(file, size) {
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
        cli.height,
        cli.width,
        if cli.preserve_ratio_resolved() {
            "true"
        } else {
            "false"
        },
        encoded
    );
}

fn write_placeholder<W: Write>(out: &mut W, cli: &Cli) {
    let name_b64 = B64.encode(b"one_pixel_black");
    let _ = write!(
        out,
        "\x1b]1337;File=name={};size={};inline=1;height={};width={};preserveAspectRatio={}:{}\x07",
        name_b64,
        ONE_PIXEL_BLACK.len(),
        cli.height,
        cli.width,
        if cli.preserve_ratio_resolved() {
            "true"
        } else {
            "false"
        },
        ONE_PIXEL_BLACK
    );
}

fn get_dimensions(path: &Path) -> Option<(usize, usize)> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_lowercase);
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

fn ls_sort(paths: &mut [PathBuf], cli: &Cli) {
    let samesort = cli.y || std::env::var("LS_SAMESORT").is_ok();
    if cli.sort_t() {
        paths.sort_by(|a, b| {
            let am = lstat(a).map_or(0, |m| m.mtime());
            let bm = lstat(b).map_or(0, |m| m.mtime());
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
    } else if cli.ctime() {
        paths.sort_by(|a, b| {
            let am = lstat(a).map_or(0, |m| m.ctime());
            let bm = lstat(b).map_or(0, |m| m.ctime());
            match bm.cmp(&am) {
                Ordering::Equal => a.cmp(b),
                o => o,
            }
        });
    } else if cli.u {
        paths.sort_by(|a, b| {
            let am = lstat(a).map_or(0, |m| m.atime());
            let bm = lstat(b).map_or(0, |m| m.atime());
            am.cmp(&bm)
        });
    } else if cli.sort_s_upper() {
        paths.sort_by(|a, b| {
            let asz = lstat(a).map_or(0, |m| m.size());
            let bsz = lstat(b).map_or(0, |m| m.size());
            match bsz.cmp(&asz) {
                Ordering::Equal => a.cmp(b),
                o => o,
            }
        });
    } else {
        paths.sort();
    }
    if cli.r {
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
    const UNITS: &[&str] = &["B", "K", "M", "G", "T", "P"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i + 1 < UNITS.len() {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{}B", bytes)
    } else if v >= 10.0 {
        format!("{:.0}{}", v, UNITS[i])
    } else {
        format!("{:.1}{}", v, UNITS[i])
    }
}

fn zoned_from_unix(time: i64) -> Zoned {
    let ts = Timestamp::from_second(time).unwrap_or_else(|_| Timestamp::now());
    ts.to_zoned(TimeZone::system())
}

fn format_time(time: i64, cli: &Cli) -> String {
    let dt = zoned_from_unix(time);
    let now = Zoned::now().timestamp().as_second();
    let fmt: &str = if let Some(f) = &cli.d_fmt {
        f.as_str()
    } else if cli.t_upper_resolved() {
        "%b %e %H:%M:%S %Y"
    } else if time + SIX_MONTHS > now && time < now + SIX_MONTHS {
        "%b %e %H:%M"
    } else {
        "%b %e  %Y"
    };
    dt.strftime(fmt).to_string()
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
        CStr::from_ptr(n).to_str().ok().map(ToString::to_string)
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
        CStr::from_ptr(n).to_str().ok().map(ToString::to_string)
    }
}

fn get_f_type(md: &fs::Metadata, cli: &Cli) -> &'static str {
    let ft = md.file_type();
    if ft.is_dir() && (cli.p || cli.f_upper) {
        return "/";
    }
    if !cli.f_upper {
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

fn sanitize_for_tty(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let c = ch as u32;
        if c < 0x20 || c == 0x7f {
            out.push('?');
        } else {
            out.push(ch);
        }
    }
    out
}

fn dot_filtered(name: &str, cli: &Cli) -> bool {
    if cli.a {
        return false;
    }
    if cli.a_upper {
        return name == "." || name == "..";
    }
    name.starts_with('.')
}

fn read_dir_split(path: &Path, cli: &Cli) -> std::io::Result<(Vec<PathBuf>, Vec<PathBuf>)> {
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if dot_filtered(&name_str, cli) {
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

fn do_ls<W: Write>(out: &mut W, parent: Option<&Path>, mut paths: Vec<PathBuf>, cli: &Cli) {
    ls_sort(&mut paths, cli);

    let show_dims = cli.show_dimensions();
    let show_unknown = cli.show_unknown();
    let long = cli.long();
    let show_blocks = cli.show_blocks();

    let mut entries: Vec<Entry> = Vec::with_capacity(paths.len());
    let mut blocks_total: u64 = 0;

    for p in paths {
        let Some(md) = lstat(&p) else {
            continue;
        };
        let dims = if show_dims && md.file_type().is_file() && md.len() > 0 {
            get_dimensions(&p)
        } else {
            None
        };

        if show_blocks || long {
            let name = entry_filename(&p, parent);
            let is_hidden_dotfile = name.starts_with('.') && !name.starts_with("..") && name != ".";
            if !md.file_type().is_dir() && (cli.a || !is_hidden_dotfile) {
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
        if show_blocks || long {
            w_blocks = w_blocks.max(e.md.blocks().to_string().len());
            w_nlink = w_nlink.max(e.md.nlink().to_string().len());
        }
        if cli.i {
            w_ino = w_ino.max(e.md.ino().to_string().len());
        }
        if long {
            w_bytes = w_bytes.max(e.md.len().to_string().len());
            let owner = if cli.n {
                e.md.uid().to_string()
            } else {
                uid_name(e.md.uid()).unwrap_or_else(|| e.md.uid().to_string())
            };
            w_owner = w_owner.max(owner.len());
            if !cli.o {
                let group = if cli.n {
                    e.md.gid().to_string()
                } else {
                    gid_name(e.md.gid()).unwrap_or_else(|| e.md.gid().to_string())
                };
                w_group = w_group.max(group.len());
            }
        }
    }

    if (show_blocks || long) && parent.is_some() && !cli.d {
        let total = if cli.k {
            blocks_total / 2
        } else {
            blocks_total
        };
        let _ = writeln!(out, "total {}", total);
    }

    for e in &entries {
        let placeholder = !e.md.file_type().is_file()
            || e.md.len() == 0
            || (show_dims && e.dims.is_none() && !show_unknown);
        if placeholder {
            write_placeholder(out, cli);
        } else {
            write_image(out, &e.path, e.md.len(), cli);
        }

        if show_dims && (w_dim_w > 0 || w_dim_h > 0) {
            let mw = w_dim_w.max(1);
            let mh = w_dim_h.max(1);
            if let Some((w, h)) = e.dims {
                let _ = write!(out, " [{:>mw$} x {:>mh$}] ", w, h, mw = mw, mh = mh);
            } else {
                let _ = write!(out, " {:>mw$}   {:>mh$}   ", "", "", mw = mw, mh = mh);
            }
        }

        if cli.i {
            let _ = write!(out, " {:>w$}", e.md.ino(), w = w_ino);
        }
        if cli.s {
            let _ = write!(out, " {:>w$}", e.md.blocks(), w = w_blocks);
        }
        if long {
            let _ = write!(out, " {}", format_mode(e.md.mode()));
            let _ = write!(out, " {:>w$}", e.md.nlink(), w = w_nlink);
            let owner = if cli.n {
                e.md.uid().to_string()
            } else {
                uid_name(e.md.uid()).unwrap_or_else(|| e.md.uid().to_string())
            };
            let _ = write!(out, " {:>w$}", owner, w = w_owner);
            if !cli.o {
                let group = if cli.n {
                    e.md.gid().to_string()
                } else {
                    gid_name(e.md.gid()).unwrap_or_else(|| e.md.gid().to_string())
                };
                let _ = write!(out, "  {:>w$}", group, w = w_group);
            }
            if cli.h {
                let _ = write!(out, "  {:>4}", format_human(e.md.len()));
            } else {
                let _ = write!(out, "  {:>w$}", e.md.len(), w = w_bytes);
            }
            let t = if cli.ctime() {
                e.md.ctime()
            } else {
                e.md.mtime()
            };
            let _ = write!(out, " {}", format_time(t, cli));
        }

        let name = sanitize_for_tty(&entry_filename(&e.path, parent));
        let _ = write!(out, " {}", name);
        let suffix = get_f_type(&e.md, cli);
        let _ = write!(out, "{}", suffix);
        let _ = writeln!(out);
    }
}

fn main() {
    // Restore default SIGPIPE behavior so writing to a closed pipe terminates
    // the process cleanly instead of triggering a Rust panic on stdout writes.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let cli = Cli::parse();
    let mut paths = cli.paths.clone();

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
                    "thmbs: {}: No such file or directory",
                    p.display()
                );
            }
            Some(md) if md.file_type().is_file() || (cli.d && !md.file_type().is_dir()) => {
                files.push(p);
            }
            Some(md) if md.file_type().is_dir() => {
                if cli.d {
                    files.push(p);
                } else {
                    dirs.push(p);
                }
            }
            Some(_) => files.push(p),
        }
    }

    ls_sort(&mut files, &cli);
    ls_sort(&mut dirs, &cli);

    if !files.is_empty() {
        do_ls(&mut out, None, files, &cli);
    }

    let mut do_newline = false;
    let mut visited: HashSet<(u64, u64)> = HashSet::new();
    let mut queue: VecDeque<PathBuf> = VecDeque::from(dirs);
    while let Some(path) = queue.pop_front() {
        if let Some(md) = lstat(&path) {
            let key = (md.dev(), md.ino());
            if !visited.insert(key) {
                continue;
            }
        }
        let (f, d) = match read_dir_split(&path, &cli) {
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
            let _ = writeln!(out, "{}:", sanitize_for_tty(&path.display().to_string()));
        }
        do_header = true;

        let mut combined = f;
        combined.extend(d.iter().cloned());
        do_ls(&mut out, Some(&path), combined, &cli);

        if cli.recurse() {
            for sub in d {
                let name = sub
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if name == "." || name == ".." {
                    continue;
                }
                queue.push_back(sub);
            }
        }
        do_newline = true;
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::*;

    fn default_cli() -> Cli {
        Cli {
            help: None,
            width: 3,
            height: 1,
            preserve_ratio: true,
            no_preserve_ratio: false,
            dimensions: false,
            no_dimensions: false,
            unknown: false,
            no_unknown: false,
            a_upper: false,
            f_upper: false,
            r_upper: false,
            t_upper: false,
            a: false,
            d: false,
            h: false,
            i: false,
            k: false,
            l: false,
            n: false,
            o: false,
            p: false,
            r: false,
            s: false,
            t: false,
            s_upper: false,
            u: false,
            y: false,
            c: false,
            u_upper: false,
            d_fmt: None,
            paths: vec![],
        }
    }

    #[test]
    fn format_human_boundaries() {
        assert_eq!(format_human(0), "0B");
        assert_eq!(format_human(1), "1B");
        assert_eq!(format_human(1023), "1023B");
        assert_eq!(format_human(1024), "1.0K");
        assert_eq!(format_human(1536), "1.5K");
        assert_eq!(format_human(10 * 1024), "10K");
        assert_eq!(format_human(1024 * 1024 - 1), "1024K");
        assert_eq!(format_human(1024 * 1024), "1.0M");
    }

    #[test]
    fn format_mode_regular_file() {
        // 0o100644 = regular file, rw-r--r--
        assert_eq!(format_mode(0o100644), "-rw-r--r--");
    }

    #[test]
    fn format_mode_directory() {
        // 0o040755 = directory, rwxr-xr-x
        assert_eq!(format_mode(0o040755), "drwxr-xr-x");
    }

    #[test]
    fn format_mode_symlink() {
        // 0o120777 = symlink
        assert_eq!(format_mode(0o120777), "lrwxrwxrwx");
    }

    #[test]
    fn format_mode_setuid() {
        // 0o104755 = regular file with setuid, rwsr-xr-x
        assert_eq!(format_mode(0o104755), "-rwsr-xr-x");
    }

    #[test]
    fn format_mode_setuid_no_x() {
        // 0o104644 = setuid without owner-x, rwSr--r--
        assert_eq!(format_mode(0o104644), "-rwSr--r--");
    }

    #[test]
    fn format_mode_sticky() {
        // 0o041777 = directory, sticky, rwxrwxrwt
        assert_eq!(format_mode(0o041777), "drwxrwxrwt");
    }

    #[test]
    fn format_mode_setgid() {
        // 0o102755 = setgid file: rwxr-sr-x
        assert_eq!(format_mode(0o102755), "-rwxr-sr-x");
    }

    #[test]
    fn format_time_known_epoch() {
        // For a fixed timestamp, just check that it produces non-empty output
        // and that the d_fmt override works.
        let mut cli = default_cli();
        cli.d_fmt = Some("%Y-%m-%d".to_string());
        // Use a SystemTime to derive seconds, demonstrating use of the import.
        let st = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let secs = st.duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        let s = format_time(secs, &cli);
        // 2023-11-14 in UTC; locally may shift one day either way, just check format shape
        assert_eq!(s.len(), 10);
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[7..8], "-");
    }

    #[test]
    fn format_time_t_upper_format() {
        let mut cli = default_cli();
        cli.t_upper = true;
        let s = format_time(0, &cli);
        // "%b %e %H:%M:%S %Y" -> ends with year (4 digits)
        let year: i32 = s[s.len() - 4..].parse().unwrap();
        assert!((1969..=1970).contains(&year));
    }

    #[test]
    fn dot_filtered_default() {
        let cli = default_cli();
        assert!(dot_filtered(".foo", &cli));
        assert!(dot_filtered("..", &cli));
        assert!(!dot_filtered("foo", &cli));
    }

    #[test]
    fn dot_filtered_a() {
        let mut cli = default_cli();
        cli.a = true;
        assert!(!dot_filtered(".foo", &cli));
        assert!(!dot_filtered("..", &cli));
        assert!(!dot_filtered("foo", &cli));
    }

    #[test]
    fn dot_filtered_a_upper() {
        let mut cli = default_cli();
        cli.a_upper = true;
        // -A: keep .foo, drop . and ..
        assert!(!dot_filtered(".foo", &cli));
        assert!(dot_filtered(".", &cli));
        assert!(dot_filtered("..", &cli));
        assert!(!dot_filtered("foo", &cli));
    }
}
