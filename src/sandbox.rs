use std::env;
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::unix;
use std::process;
use nix::mount;
use nix::sched;
use nix::sys::stat::{self, SFlag};
use nix::sys::wait;
use nix::unistd::{self, Uid, Gid};

pub fn init() {
    let host_uid = unistd::geteuid();
    let host_gid = unistd::getegid();
    let guest_uid = Uid::from_raw(1000);
    let guest_gid = Gid::from_raw(1000);
    sched::unshare(sched::CLONE_NEWNS | sched::CLONE_NEWUTS |
                   sched::CLONE_NEWIPC | sched::CLONE_NEWUSER |
                   sched::CLONE_NEWPID | sched::CLONE_NEWNET).unwrap();
    write_file("/proc/self/uid_map", &format!("{} {} 1", guest_uid, host_uid));
    write_file("/proc/self/setgroups", "deny");
    write_file("/proc/self/gid_map", &format!("{} {} 1", guest_gid, host_gid));
    unistd::setresuid(guest_uid, guest_uid, guest_uid).unwrap();
    unistd::setresgid(guest_gid, guest_gid, guest_gid).unwrap();
    unistd::sethostname("icebox").unwrap();
    match unistd::fork().unwrap() {
        unistd::ForkResult::Parent { child } => {
            wait::waitpid(child, None).unwrap();
            process::exit(0)
        },
        unistd::ForkResult::Child => (),
    }
    // TODO(iceboy): Use tempdir.
    let mount_dir = "/tmp";
    mount::mount(Some("sandbox_root"),
                 mount_dir,
                 Some("tmpfs"),
                 mount::MS_NOSUID,
                 None as Option<&[u8]>).unwrap();
    env::set_current_dir(mount_dir).unwrap();
    fs::create_dir("proc").unwrap();
    mount::mount(Some("sandbox_proc"),
                 "proc",
                 Some("proc"),
                 mount::MS_NOSUID,
                 None as Option<&[u8]>).unwrap();
    fs::create_dir("dev").unwrap();
    bind_dev("/dev/null", "dev/null");
    bind_dev("/dev/urandom", "dev/urandom");
    fs::create_dir("tmp").unwrap();
    mount::mount(Some("sandbox_tmp"),
                 "tmp",
                 Some("tmpfs"),
                 mount::MS_NOSUID,
                 Some("size=16m,nr_inodes=4k")).unwrap();
    bind_or_link("/bin", "bin");
    bind_or_link("/etc/alternatives", "etc/alternatives");
    bind_or_link("/lib", "lib");
    bind_or_link("/lib64", "lib64");
    bind_or_link("/usr/bin", "usr/bin");
    bind_or_link("/usr/include", "usr/include");
    bind_or_link("/usr/lib", "usr/lib");
    bind_or_link("/usr/lib64", "usr/lib64");
    bind_or_link("/usr/libexec", "usr/libexec");
    bind_or_link("/usr/share", "usr/share");
    bind_or_link("/var/lib/ghc", "var/lib/ghc");
    // TODO(iceboy): in & out dir.
    write_file("etc/passwd", "icebox:x:1000:1000:icebox:/:/bin/bash\n");
    fs::create_dir("old_root").unwrap();
    unistd::pivot_root(".", "old_root").unwrap();
    mount::umount2("old_root", mount::MNT_DETACH).unwrap();
    fs::remove_dir("old_root").unwrap();
    mount::mount(Some("/"),
                 "/",
                 None as Option<&[u8]>,
                 mount::MS_BIND | mount::MS_REC | mount::MS_NOSUID,
                 None as Option<&[u8]>).unwrap();
    mount::mount(Some("/"),
                 "/",
                 None as Option<&[u8]>,
                 mount::MS_BIND | mount::MS_REMOUNT | mount::MS_RDONLY |
                 mount::MS_REC | mount::MS_NOSUID,
                 None as Option<&[u8]>).unwrap();
}

fn write_file(path: &str, data: &str) {
    File::create(path).unwrap().write_all(data.as_bytes()).unwrap();
}

fn bind_dev(source: &str, target: &str) {
    stat::mknod(
        target, SFlag::empty(), stat::S_IRUSR | stat::S_IWUSR, 0).unwrap();
    mount::mount(Some(source),
                 target,
                 None as Option<&[u8]>,
                 mount::MS_BIND | mount::MS_NOSUID,
                 None as Option<&[u8]>).unwrap();
}

fn bind_or_link(source: &str, target: &str) {
    let file_type = match fs::symlink_metadata(source) {
        Ok(attr) => attr.file_type(),
        Err(ref e) if e.kind() == io::ErrorKind::NotFound => return,
        Err(e) => panic!("{:?}", e),
    };
    if file_type.is_dir() {
        fs::create_dir_all(target).unwrap();
        mount::mount(Some(source),
                     target,
                     None as Option<&[u8]>,
                     mount::MS_BIND | mount::MS_REC | mount::MS_NOSUID,
                     None as Option<&[u8]>).unwrap();
    } else if file_type.is_symlink() {
        let link = fs::read_link(source).unwrap();
        unix::fs::symlink(link, target).unwrap();
    }
}
