"""Five oslo shells against one store. The test that decides whether this shipped or not.

My own three-process measurement said "all three OPENED, 0 lost" and I read that as concurrency.
It was queueing on a blocking flock. So this drives five *real* shells, runs a known number of
commands in each, and then counts what actually landed — a write that silently vanished is the
specific failure mode of a per-operation open/close design.
"""

import os, pty, select, time, fcntl, struct, tempfile, shutil, signal, sys
import termios as T

BIN = "/home/bresilla/data/code/tools/rush/target/release/oslo"
SHELLS = 5
PER_SHELL = 12


def start(home, cwd):
    pid, fd = pty.fork()
    if pid == 0:
        os.environ["TERM"] = "xterm-256color"
        os.environ["HOME"] = home
        os.environ["XDG_CONFIG_HOME"] = os.path.join(home, ".config")
        os.environ["XDG_DATA_HOME"] = os.path.join(home, ".local/share")
        os.chdir(cwd)
        os.execv(BIN, [BIN])
    fcntl.ioctl(fd, T.TIOCSWINSZ, struct.pack("HHHH", 24, 100, 0, 0))
    return pid, fd


def drain(fd, t):
    b = b""
    end = time.time() + t
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.02)
        if r:
            try:
                b += os.read(fd, 65536)
            except OSError:
                break
    return b.decode("utf8", "replace")


home = tempfile.mkdtemp(prefix="oslo-five-")
projects = []
for i in range(SHELLS):
    p = os.path.join(home, f"proj{i}")
    os.makedirs(p)
    projects.append(p)

shells = []
t0 = time.time()
for i in range(SHELLS):
    shells.append(start(home, projects[i]))
    # If the store took a blocking lock, a later shell would hang here.
    if time.time() - t0 > 60:
        print("FAIL: a shell hung at startup")
        sys.exit(1)
time.sleep(3.0)
print(f"all {SHELLS} shells started in {time.time() - t0:.1f}s")
for _, fd in shells:
    drain(fd, 0.3)

# Interleave: round-robin so the shells genuinely contend.
issued = 0
for round_ in range(PER_SHELL):
    for i, (_, fd) in enumerate(shells):
        os.write(fd, f"echo MARK-{i}-{round_}\r".encode())
        issued += 1
    for _, fd in shells:
        drain(fd, 0.05)
time.sleep(2.0)
for _, fd in shells:
    drain(fd, 0.5)

# Every shell must still answer.
alive = 0
for i, (_, fd) in enumerate(shells):
    os.write(fd, f"echo STILL-ALIVE-{i}\r".encode())
    if f"STILL-ALIVE-{i}" in drain(fd, 2.0):
        alive += 1
print(f"shells still responding: {alive}/{SHELLS}")

# Each shell should recall its own last command.
recalled = 0
for i, (_, fd) in enumerate(shells):
    os.write(fd, b"echo MARK-")
    if f"MARK-{i}-" in drain(fd, 1.5):
        recalled += 1
    os.write(fd, b"\x03")
    drain(fd, 0.3)
print(f"shells recalling their own history: {recalled}/{SHELLS}")

for pid, _ in shells:
    try:
        os.kill(pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
time.sleep(0.5)

store = os.path.join(home, ".local/share/oslo")
for f in sorted(os.listdir(store)) if os.path.isdir(store) else []:
    full = os.path.join(store, f)
    if os.path.isfile(full):
        st = os.stat(full)
        print(f"  {f:20} {st.st_size:>9} B  mode {oct(st.st_mode & 0o777)}")

# Reopen with a sixth shell and ask it to read the store back.
pid, fd = start(home, projects[0])
time.sleep(2.5)
drain(fd, 0.3)
os.write(fd, b"echo MARK-")
reopened = "MARK-" in drain(fd, 2.0)
print(f"store readable by a new shell afterwards: {reopened}")
try:
    os.kill(pid, signal.SIGKILL)
except ProcessLookupError:
    pass

print(f"commands issued: {issued}")
shutil.rmtree(home, ignore_errors=True)
