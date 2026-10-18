//! Windows process-job regression fixture; compiled only by its integration test.
fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--version") => println!("dprint 0.63.0"),
        Some("--hold-pipes") => std::thread::sleep(std::time::Duration::from_secs(60)),
        _ => {
            // The CLI-shaped parent exits while a child retains inherited pipes.
            // The backend must terminate the job, even after the parent is gone.
            std::process::Command::new(std::env::current_exe().unwrap())
                .arg("--hold-pipes")
                .stdin(std::process::Stdio::null())
                .spawn()
                .unwrap();
        }
    }
}
