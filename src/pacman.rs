use crate::args::Args;
use alpm::{
    Alpm, AnyDownloadEvent, AnyEvent, DownloadEvent, DownloadResult, Event, LogLevel, Package,
    SigLevel,
};
use alpm_utils::DbListExt;
use alpm_utils::Targ;
use anyhow::{Context, Result};

pub fn alpm_init(args: &Args) -> Result<Alpm> {
    let mut conf =
        pacmanconf::Config::with_opts(None, args.config.as_deref(), args.root.as_deref())?;
    if let Some(dbpath) = args.dbpath.clone() {
        conf.db_path = dbpath;
    }
    let mut alpm = Alpm::new(conf.root_dir.as_str(), conf.db_path.as_str()).with_context(|| {
        format!(
            "failed to initialize alpm (root: {}, dbpath: {})",
            conf.root_dir.as_str(),
            conf.db_path,
        )
    })?;

    if args.filedb {
        alpm.set_dbext(".files");
    }

    alpm.set_dl_cb((), download_cb);
    alpm.set_log_cb((), log_cb);
    alpm.set_event_cb((), event_cb);

    alpm_utils::configure_alpm(&mut alpm, &conf)?;

    if let Some(dir) = args.cachedir.as_deref() {
        alpm.add_cachedir(dir)?;
    } else {
        let tmp = std::env::temp_dir()
            .join("paccat")
            .to_str()
            .context("tempdir is not a str")?
            .to_string();
        alpm.add_cachedir(tmp)?;
    }

    if args.refresh > 0 {
        eprintln!("synchronising package databases...");
        alpm.syncdbs_mut().update(args.refresh > 1)?;
    }

    for db in alpm.syncdbs() {
        db.is_valid()
            .with_context(|| format!("database {}{} is not valid", db.name(), alpm.dbext()))?
    }

    Ok(alpm)
}

pub fn get_dbpkg<'a>(alpm: &'a Alpm, target_str: &str, localdb: bool) -> Result<Package<'a>> {
    let pkg = if localdb {
        alpm.localdb().pkg(target_str).ok()
    } else {
        let target = Targ::from(target_str);
        alpm.syncdbs().find_target_satisfier(target)
    };
    let pkg = pkg.with_context(|| format!("could not find package: {}", target_str))?;
    Ok(pkg)
}

pub fn verify_packages<'a, I>(alpm: &Alpm, siglevel: SigLevel, files: I) -> Result<()>
where
    I: IntoIterator<Item = &'a str>,
{
    if !siglevel.contains(SigLevel::PACKAGE) {
        return Ok(());
    }

    for file in files {
        if let Err(e) = alpm
            .pkg_load(file, false, alpm.remote_file_siglevel())?
            .check_signature()
        {
            if e == alpm::Error::SigMissing && siglevel.contains(SigLevel::PACKAGE_OPTIONAL) {
                continue;
            }

            Err(e).with_context(|| format!("failed to verify package {}", file))?;
        }
    }

    Ok(())
}

pub fn get_download_url(pkg: Package) -> Result<String> {
    let server = pkg
        .db()
        .unwrap()
        .servers()
        .first()
        .ok_or(alpm::Error::ServerNone)?;
    let url = format!("{}/{}", server, pkg.filename());
    Ok(url)
}

fn download_cb(file: &str, event: AnyDownloadEvent, _: &mut ()) {
    if file.ends_with(".sig") {
        return;
    }

    if let DownloadEvent::Completed(c) = event.event() { match c.result {
        DownloadResult::Success => eprintln!("{} downloaded", file),
        DownloadResult::UpToDate => eprintln!("{} is up to date", file),
        DownloadResult::Failed => eprintln!("{} failed to download", file),
    } }
}

fn log_cb(level: LogLevel, msg: &str, _: &mut ()) {
    match level {
        LogLevel::WARNING => eprint!("warning: {}", msg),
        LogLevel::ERROR => eprint!("error: {}", msg),
        _ => (),
    }
}

fn event_cb(event: AnyEvent, _: &mut ()) {
    if let Event::DatabaseMissing(e) = event.event() {
        eprintln!(
            "database file for {} does not exist (use pacman to download)",
            e.dbname()
        );
    }
}
