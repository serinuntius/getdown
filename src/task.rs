use std::{
    cell::RefCell,
    error::Error,
    fs::{self, File},
    io::{self, BufWriter, Read, Write},
    path::Path,
    rc::Rc,
    str::from_utf8,
    sync::{Arc, Mutex},
};

use futures::StreamExt;
use indicatif::{MultiProgress, ProgressBar};
use reqwest::{
    header::{HeaderMap, HeaderValue},
    Url,
};

#[derive(Clone, Default)]
pub struct Range {
    low: u64,
    high: u64,
}

impl Range {
    fn from(index: u64, procs: u64, task_size: u64, content_length: u64) -> Self {
        let low = task_size * index;
        if index == procs - 1 {
            return Range {
                low,
                high: content_length,
            };
        }
        Range {
            low,
            high: low + task_size - 1,
        }
    }
    fn bytes_range(&self) -> String {
        format!("bytes={}-{}", self.low, self.high)
    }

    pub fn abs(&self) -> u64 {
        self.high - self.low
    }
}

#[derive(Clone)]
pub struct Task {
    pub id: u64,
    pub range: Range,
    pub url: Url,
    pub procs: u64,
    pub file_name: String,
}
impl Task {
    fn default() -> Self {
        Task {
            id: 0,
            range: Range::default(),
            url: Url::parse("").unwrap(),
            procs: 0,
            file_name: "".to_string(),
        }
    }
}

impl Task {
    pub async fn download(&self, bar: ProgressBar) -> Result<(), Box<dyn Error>> {
        bar.inc(1);
        bar.println("download start!");

        let mut headers = HeaderMap::new();
        headers.insert(
            "Range",
            HeaderValue::from_str(&self.range.bytes_range()).expect("failed to set header"),
        );

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()?;

        let resp = client.get(self.url.clone()).send().await?;

        let file = File::options()
            .write(true)
            .create_new(true)
            .open(self.dest_path())?;
        let mut buffer = BufWriter::new(file);

        let mut stream = resp.bytes_stream();
        while let Some(item) = stream.next().await {
            let b: Result<Vec<u8>, _> = item?.bytes().collect();
            let bytes = buffer.write(&b.unwrap()).expect("failed");
            bar.inc(bytes.try_into().unwrap());
            buffer.flush().expect("failed to flush");
        }
        bar.finish();

        Ok(())
    }

    pub fn dest_path(&self) -> String {
        format!("{}.{}.{}", self.file_name, self.procs, self.id)
    }
}

pub struct TaskConfig {
    pub procs: u64,
    pub task_size: u64,
    pub content_length: u64,
    pub url: Url,
    pub file_name: String,
}

impl TaskConfig {
    pub fn assign_tasks(&self) -> Vec<Task> {
        let mut tasks: Vec<Task> = vec![];

        for i in 0..self.procs {
            let mut range = Range::from(i, self.procs, self.task_size, self.content_length);

            let f = format!("{}.{}.{}", self.file_name, self.procs, i);
            let part_name = Path::new(&f);

            if let Ok(metadata) = fs::metadata(part_name) {
                let file_size = metadata.len();
                if (i == self.procs - 1 && file_size == range.high - range.low)
                    || (file_size == self.task_size)
                {
                    continue;
                }

                range.low += file_size;
            }

            tasks.push(Task {
                id: i,
                procs: self.procs,
                range,
                url: self.url.clone(),
                file_name: self.file_name.clone(),
            });
        }

        tasks
    }
}
