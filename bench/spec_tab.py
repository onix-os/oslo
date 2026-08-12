"""How long a Tab takes on a command that carries a completion spec.

`tab.py` measures a Tab on a *command name*, which is the command index and the fuzzy matcher. This
one measures the other path: `git comm<TAB>` walks the spec registry's subcommand tree, which is the
code that changed when specs stopped being `&'static str` and started owning their strings.

Min-of-N on a quiet machine, interleaved with nothing else. The tree is measurably layout-sensitive,
so a single run either way says nothing.
"""

import os, pty, select, time, fcntl, struct, tempfile, shutil, signal, statistics
import termios as T

BIN = "/home/bresilla/data/code/tools/rush/target/release/oslo"


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


def wait_quiet(fd, timeout=5.0):
    """Time until the redraw stops arriving — a Tab repaints, it does not print a new prompt."""
    start = time.time()
    last = None
    while time.time() - start < timeout:
        r, _, _ = select.select([fd], [], [], 0.005)
        if r:
            try:
                os.read(fd, 65536)
            except OSError:
                break
            last = time.time()
        elif last is not None:
            return last - start
    return None


home = tempfile.mkdtemp(prefix="oslo-spec-")
pid, fd = session(home)
time.sleep(2.5)
while select.select([fd], [], [], 0.05)[0]:
    os.read(fd, 65536)

times = []
for _ in range(40):
    os.write(fd, b"git comm\t")
    took = wait_quiet(fd)
    if took is not None:
        times.append(took * 1000)
    # Back to an empty line, so every sample starts from the same place.
    os.write(fd, b"\x15")
    wait_quiet(fd, timeout=0.5)

times.sort()
print("--- Tab on `git comm` (spec registry walk + draw)")
print(f"samples          : {len(times)}")
print(f"min              : {min(times):.2f} ms")
print(f"median           : {statistics.median(times):.2f} ms")
print(f"p90              : {times[int(len(times) * 0.9)]:.2f} ms")

os.kill(pid, signal.SIGKILL)
shutil.rmtree(home, ignore_errors=True)
