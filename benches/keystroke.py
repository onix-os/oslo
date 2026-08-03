"""Does anything touch .git while you type? The report says yes, on every keystroke."""

import os, pty, select, time, fcntl, struct, tempfile, shutil, signal
import termios as T

BIN = "/home/bresilla/data/code/tools/rush/target/release/oslo"
REPO = "/home/bresilla/data/code/tools/rush"

home = tempfile.mkdtemp(prefix="oslo-ks-")
trace = os.path.join(home, "trace")

pid, fd = pty.fork()
if pid == 0:
    os.environ["TERM"] = "xterm-256color"
    os.environ["HOME"] = home
    os.environ["XDG_CONFIG_HOME"] = os.path.join(home, ".config")
    os.environ["XDG_DATA_HOME"] = os.path.join(home, ".local/share")
    os.chdir(REPO)
    os.execvp(
        "strace",
        ["strace", "-f", "-e", "trace=openat,statx,newfstatat,execve", "-o", trace, BIN],
    )

fcntl.ioctl(fd, T.TIOCSWINSZ, struct.pack("HHHH", 24, 120, 0, 0))


def drain(t):
    end = time.time() + t
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.02)
        if r:
            try:
                os.read(fd, 65536)
            except OSError:
                return


def lines():
    try:
        return open(trace, errors="replace").readlines()
    except FileNotFoundError:
        return []


drain(4.0)
before = len(lines())

typed = "echo hello world"
start = time.time()
for ch in typed:
    os.write(fd, ch.encode())
    time.sleep(0.04)
drain(1.0)
elapsed = time.time() - start

after = lines()[before:]
git = [l for l in after if ".git" in l]
execs = [l for l in after if "execve" in l]

print(f"keystrokes                 : {len(typed)}")
print(f"syscalls while typing      : {len(after)}  ({len(after)/len(typed):.1f} per keystroke)")
print(f"  touching .git            : {len(git)}")
print(f"  execve (subprocesses)    : {len(execs)}")
if git:
    print("  sample:", git[0].strip()[:110])

os.write(fd, b"\x03")
drain(0.3)
os.kill(pid, signal.SIGKILL)
shutil.rmtree(home, ignore_errors=True)
