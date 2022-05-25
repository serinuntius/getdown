pub mod task;

use std::{
    cell::RefCell,
    fs::{self, File},
    io::{self, BufWriter},
    path::Path,
    process::exit,
    rc::Rc,
    sync::{Arc, Mutex},
    thread,
};

use clap::Parser;
use indicatif::{MultiProgress, ProgressBar};
use once_cell::sync::Lazy;
use reqwest::{header::ACCEPT_RANGES, Client};
use task::{Range, Task, TaskConfig};
use tokio::task::spawn;
use url::Url;

use futures::future;

#[derive(Parser)]
#[clap(
    author = "created by serinuntius",
    version,
    about = "resumable, fastest downloader"
)]
struct AppArg {
    #[clap(help = "download url")]
    url: String,

    #[clap(
        short = 'p',
        long = "proc",
        help = "the number of connections for a single URL (default 2)"
    )]
    proc: Option<u64>,

    #[clap(
        short = 'v',
        long = "verbose",
        help = "Make the operation more talkative"
    )]
    verbose: bool,
}

struct TargetInfo {
    url: Url,
    file_name: String,
    content_length: u64,
}

async fn get_target_info(url: Url) -> Result<TargetInfo, Box<dyn std::error::Error>> {
    let client = Client::new();

    println!("url: {}", url.clone());
    let res = client.get(url.clone()).send().await?;

    let content_length = res
        .content_length()
        .expect("get_target_info(): failed to get content_length()");

    if content_length == 0 {
        println!("res: {:?}", res.headers());
        println!("url: {:?}", url);

        eprintln!("get_target_info(): invalid content length");
        exit(1);
    }

    println!("res: {:?}", res.headers());

    if res
        .headers()
        .get(ACCEPT_RANGES)
        .expect("get_target_info(): failed to get ACCEPT_RANGES")
        != "bytes"
    {
        eprintln!(
            "get_target_info(): failed to get ACCEPT_RANGES. headers: {:?}",
            res.headers()
        );
        exit(1);
    }

    let _url = res.url().clone();

    if url.clone() != _url {
        let u = _url
            .path_segments()
            .unwrap()
            .into_iter()
            .last()
            .unwrap()
            .to_string();

        Ok(TargetInfo {
            url: _url,
            file_name: u,
            content_length,
        })
    } else {
        let u = url
            .path_segments()
            .unwrap()
            .into_iter()
            .last()
            .unwrap()
            .to_string();

        Ok(TargetInfo {
            content_length,
            url: url.clone(),
            file_name: u,
        })
    }
}

// #[derive(Clone)]
// pub struct DownloadConfig {
//     pub file_name: &'static String,
//     pub content_length: &'static u64,
//     pub tasks: &'static Vec<Task>,
//     pub partial_dir: &'static String,
// }

#[derive(Clone, Default)]
pub struct DownloadConfig {
    pub file_name: String,
    pub content_length: u64,
    pub tasks: Vec<Task>,
    pub partial_dir: String,
}

// impl DownloadConfig {
//     fn new(
//         file_name: String,
//         content_length: u64,
//         tasks: Vec<Task>,
//         partial_dir: String,
//     ) -> 'static Self {
//         let c = DownloadConfig {
//             file_name,
//             content_length,
//             tasks,
//             partial_dir,
//         };
//         return &c;
//     }
// }

async fn download_parallel(option: AppArg) -> Result<(), Box<dyn std::error::Error>> {
    let mut tokio_tasks = vec![];
    let mp = MultiProgress::new();
    // let rc = Arc::new(Mutex::new(mp));

    unsafe {
        for c in &CONFIG.tasks {
            let range = &c.range;
            let pb = mp.add(ProgressBar::new(range.abs()));
            pb.set_message("dl: ");

            let t = spawn(async move {
                c.download(pb).await.expect("failed to download");
            });
            tokio_tasks.push(t);
        }
    }

    future::join_all(tokio_tasks.into_iter()).await;

    mp.join_and_clear().unwrap();

    concat_files(option).await?;
    Ok(())
}

async fn concat_files(_option: AppArg) -> Result<(), Box<dyn std::error::Error>> {
    println!("concat files...");

    unsafe {
        let file_name = &CONFIG.file_name;

        let mut target_file = File::options()
            .create_new(true)
            .write(true)
            .open(file_name)?;

        for task in &CONFIG.tasks {
            let name = task.dest_path();
            let mut f = File::open(name.clone())?;

            io::copy(&mut f, &mut target_file)?;

            fs::remove_file(name.clone())?;
        }
    }

    Ok(())
}

fn get_partial_dirname(dir_name: String, file_name: String, procs: u64) -> String {
    if dir_name.is_empty() || dir_name == "/" {
        return format!("_{}.{}", dir_name, procs);
    }

    return Path::new(&dir_name)
        .join(format!("_{}.{}", file_name, procs))
        .display()
        .to_string();
}

static mut CONFIG: Lazy<DownloadConfig> = Lazy::new(|| DownloadConfig::default());

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = AppArg::parse();

    let proc = cli.proc.unwrap_or(2);

    if cli.verbose {
        println!("proc: {}", proc);
        println!("url: {}", cli.url);
    }

    let url = match Url::parse(&cli.url) {
        Ok(it) => it,
        Err(err) => {
            eprintln!("failed to parse url('{}'), err: {}", &cli.url, err);
            exit(1)
        }
    };

    let target_info = match get_target_info(url).await {
        Ok(it) => it,
        Err(err) => {
            eprintln!("error: {}", err);
            // exit(1);
            panic!("panic");
        }
    };

    let task_config = TaskConfig {
        procs: proc.to_owned(),
        task_size: (target_info.content_length / proc),
        content_length: target_info.content_length,
        url: target_info.url,
        file_name: target_info.file_name.clone(),
    };

    let tasks = task_config.assign_tasks();

    let partial_dir = get_partial_dirname("".to_string(), target_info.file_name, proc);

    // let config = DownloadConfig {
    //     file_name: task_config.file_name.to_string(),
    //     content_length: target_info.content_length,
    //     tasks,
    //     partial_dir,
    // };

    // let c = config.clone();

    unsafe {
        CONFIG.file_name = task_config.file_name;
        CONFIG.content_length = target_info.content_length;
        CONFIG.tasks = tasks;
        CONFIG.partial_dir = partial_dir;
    }

    let a = download_parallel(cli).await?;

    // let resp = client.head(url).await?.text().await?;

    // println!("resp: {:?}", resp);

    Ok(())
}
