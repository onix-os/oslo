"""How long from pressing Enter to the next prompt, interactively.

This is what the `block_on` claim is really about: the history and tracking writes happen after the
command and before the prompt returns, so if they cost anything a person feels it here and nowhere
else. Measured against a warm store, because an empty one is not the case that matters.
"""

import os, pty, select, time, fcntl, struct, tempfile, shutil, signal, statistics
import termios as T

BIN = "/home/bresilla/data/code/tools/rush/target/release/oslo"
MARK = "\x1b]133;A"  # OSC 133 prompt-start: the shell telling us the prompt is up


def session(home):
    pid, fd = pty.fork()
    if pid == 0:
        os.environ["TERM"] = "xterm-256color"
        os.environ["HOME"] = home
        os.environ["XDG_CONFIG_HOME"] = os.path.join(home, ".config")
        os.environ["XDG_DATA_HOME"] = os.path.join(home, ".local/share")
        os.chdir(home)
        os.execv(BIN, [BIN])
    fcntl.ioctl(fd, T.TIOCSWINSZ, struct.pack("HHHH", 24, 100, 0, 0))
    return pid, fd


def wait_prompt(fd, timeout=5.0):
    """Wait for the next prompt-start mark; return how long it took."""
    start = time.time()
    buf = b""
    while time.time() - start < timeout:
        r, _, _ = select.select([fd], [], [], 0.01)
        if r:
            try:
                buf += os.read(fd, 65536)
            except OSError:
                break
            if MARK.encode() in buf:
                return time.time() - start
    return None


home = tempfile.mkdtemp(prefix="oslo-rt-")
pid, fd = session(home)
time.sleep(2.5)
while select.select([fd], [], [], 0.05)[0]:
    os.read(fd, 65536)

# Warm the stores so we are timing steady state, not first-write setup.
for i in range(40):
    os.write(fd, f"true {i}\r".encode())
    wait_prompt(fd)

times = []
for i in range(60):
    os.write(fd, b"true\r")
    took = wait_prompt(fd)
    if took is not None:
        times.append(took * 1000)

times.sort()
print(f"samples          : {len(times)}")
print(f"median           : {statistics.median(times):.2f} ms")
print(f"p90              : {times[int(len(times) * 0.9)]:.2f} ms")
print(f"max              : {max(times):.2f} ms")

os.kill(pid, signal.SIGKILL)
shutil.rmtree(home, ignore_errors=True)
