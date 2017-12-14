pub mod msg;
pub mod helpers;
pub mod error;
pub mod genmsg;

use std::env;
use std::fs::File;
use std::io::Write;
use std::path::Path;

#[macro_export]
macro_rules! rosmsg_main {
    ($($msg:expr),*)=> {
        fn main() {
            $crate::build_tools::depend_on_messages(&[
            $(
                $msg,
            )*
            ]);
        }
    }
}

#[macro_export]
macro_rules! rosmsg_include {
    () => {include!(concat!(env!("OUT_DIR"), "/msg.rs"));}
}

pub fn depend_on_messages(messages: &[&str]) {
    let cmake_paths = env::var("CMAKE_PREFIX_PATH")
        .unwrap_or(String::new())
        .split(":")
        .filter_map(append_share_folder)
        .collect::<Vec<String>>();
    let extra_paths = env::var("ROSRUST_MSG_PATH")
        .unwrap_or(String::new())
        .split(":")
        .map(String::from)
        .collect::<Vec<String>>();
    let paths = cmake_paths
        .iter()
        .chain(extra_paths.iter())
        .map(|v| v.as_str())
        .collect::<Vec<&str>>();
    let output = genmsg::depend_on_messages(paths.as_slice(), messages).unwrap();
    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("msg.rs");
    let mut f = File::create(&dest_path).unwrap();
    write!(f, "{}", output).unwrap();
}

fn append_share_folder(path: &str) -> Option<String> {
    Path::new(path).join("share").to_str().map(|v| v.to_owned())
}
