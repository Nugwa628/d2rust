"""Shared runner for the playcheck checks: one isolated, hidden `play --offscreen` game.

Every run gets its own user folder (`D2RUST_HOME`: saves, settings, caches),
log folder and perf file, so runs never touch the player's Documents folder
or each other, and several runs can go at once. Settings come from a
`settings.toml` written into that user folder before the game starts (the
game reads `$D2RUST_HOME/settings.toml` for a `--new` character), which is
how a check picks the frame limit, VSync, animation frames or text mode.

On Linux without a display the run gets its own Xvfb server (`Xvfb :N`, no
TCP, no xauth needed), so the hidden window has somewhere to live; the
picture comes from whatever Vulkan adapter the machine has (llvmpipe on the
CI server). On Windows the window is hidden by `--offscreen` itself. Nothing
here moves the real mouse, takes focus or opens a network port.

`Control` speaks the `play --control stdio` protocol (`autoplay-1` JSON
lines; the game's log shares stdout, so only lines starting with `{` are
replies).
"""
import json
import os
import shutil
import subprocess
import sys
import threading
import time
from pathlib import Path

# The settings file format version the game writes today (app/config.rs).
SETTINGS_VERSION = 7

# Display numbers handed out in this process: runs on several threads must
# not pick the same free number before either server has made its socket.
_DISPLAYS = threading.Lock()
_NEXT_DISPLAY = [100 + os.getpid() % 500]


def settings_toml(video):
    """A minimal settings file: the version and the given `[video]` keys.

    Missing keys keep the game's defaults. Values are bools, ints or
    strings."""
    lines = [f"version = {SETTINGS_VERSION}", "[video]"]
    for key, value in video.items():
        if isinstance(value, bool):
            text = "true" if value else "false"
        elif isinstance(value, int):
            text = str(value)
        else:
            text = json.dumps(str(value))
        lines.append(f"{key} = {text}")
    return "\n".join(lines) + "\n"


class Display:
    """A private Xvfb display on Linux when there is none; a no-op elsewhere."""

    def __init__(self, size="1920x1080"):
        # The screen must hold the hidden window: a window larger than the
        # screen is cut to it, and the shots with it.
        w, h = (int(n) for n in size.lower().split("x"))
        self.size = f"{max(w, 1920)}x{max(h, 1080)}"
        self.proc = None
        self.name = None

    def __enter__(self):
        if os.name == "nt" or (os.environ.get("DISPLAY") and not os.environ.get("PLAYCHECK_XVFB")):
            return self
        xvfb = shutil.which("Xvfb")
        if not xvfb:
            raise RuntimeError("no DISPLAY and no Xvfb on PATH")
        # A free display number: no lock file and no socket for it, and
        # ours: the server we started is still running once the socket is
        # there.
        for _ in range(64):
            with _DISPLAYS:
                n = _NEXT_DISPLAY[0]
                _NEXT_DISPLAY[0] = 100 + (n - 99) % 1000
            if Path(f"/tmp/.X{n}-lock").exists() or Path(f"/tmp/.X11-unix/X{n}").exists():
                continue
            proc = subprocess.Popen([xvfb, f":{n}", "-screen", "0", f"{self.size}x24",
                                     "-nolisten", "tcp"],
                                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            for _ in range(50):
                if proc.poll() is not None:
                    break
                if Path(f"/tmp/.X11-unix/X{n}").exists():
                    time.sleep(0.2)
                    if proc.poll() is None:
                        self.proc, self.name = proc, f":{n}"
                        return self
                    break
                time.sleep(0.1)
            if proc.poll() is None:
                proc.kill()
        raise RuntimeError("could not start Xvfb")

    def env(self, env):
        if self.name:
            env["DISPLAY"] = self.name
        return env

    def __exit__(self, *exc):
        if self.proc:
            self.proc.terminate()
            try:
                self.proc.wait(5)
            except subprocess.TimeoutExpired:
                self.proc.kill()


class Run:
    """One offscreen game in `folder` (created; must not exist yet)."""

    def __init__(self, exe, folder, args, video=None, size="1280x720", game_dir=None,
                 perf=True, extra_env=None):
        self.exe = Path(exe)
        self.folder = Path(folder)
        self.folder.mkdir(parents=True, exist_ok=False)
        self.home = self.folder / "user"
        (self.home / "saves").mkdir(parents=True)
        self.logs = self.folder / "logs"
        self.perf = self.folder / "perf.json" if perf else None
        if video:
            (self.home / "settings.toml").write_text(settings_toml(video), encoding="utf-8")
        self.size = size
        self.command = [str(self.exe), "play", *args, "--offscreen", "--no-sound", "--size", size]
        if game_dir:
            self.command += ["--game-dir", str(game_dir)]
        self.env = dict(os.environ, D2RUST_HOME=str(self.home), D2RUST_LOG_DIR=str(self.logs))
        self.env.pop("D2_PERF_OUT", None)
        if self.perf:
            self.env["D2_PERF_OUT"] = str(self.perf)
        if extra_env:
            self.env.update(extra_env)
        self.proc = None
        self.display = None

    def start(self, control=False):
        """Starts the game; with `control`, stdin/stdout carry the control channel."""
        self.display = Display(self.size).__enter__()
        command = self.command + (["--control", "stdio"] if control else [])
        (self.folder / "command.json").write_text(json.dumps(command) + "\n", encoding="utf-8")
        self.log = (self.folder / "stdout.log").open("w", encoding="utf-8")
        self.proc = subprocess.Popen(
            command, env=self.display.env(dict(self.env)),
            stdin=subprocess.PIPE if control else subprocess.DEVNULL,
            stdout=subprocess.PIPE if control else self.log, stderr=subprocess.STDOUT,
            text=True, encoding="utf-8", errors="replace", bufsize=1)
        return self.proc

    def wait(self, timeout):
        """The exit code; None when the game had to be stopped after `timeout` s."""
        try:
            return self.proc.wait(timeout)
        except subprocess.TimeoutExpired:
            return None
        finally:
            self.close()

    def close(self):
        if self.proc and self.proc.poll() is None:
            self.proc.kill()
            self.proc.wait()
        if self.display:
            self.display.__exit__(None, None, None)
            self.display = None
        if getattr(self, "log", None):
            self.log.close()

    def perf_report(self):
        if not self.perf or not self.perf.is_file():
            return None
        return json.loads(self.perf.read_text(encoding="utf-8"))

    def files_written(self, ignore=("saves", "logs", "settings.toml")):
        """Files the game created in its user folder, relative paths, sorted."""
        found = []
        for path in self.home.rglob("*"):
            rel = path.relative_to(self.home).as_posix()
            if path.is_file() and rel.split("/")[0] not in ignore:
                found.append(rel)
        return sorted(found)


class Control:
    """The `--control stdio` channel of a started `Run`."""

    def __init__(self, run, timeout=180):
        self.run = run
        self.timeout = timeout
        self.hello = self._reply()

    def _reply(self):
        deadline = time.monotonic() + self.timeout
        out = self.run.proc.stdout
        while time.monotonic() < deadline:
            line = out.readline()
            if not line:
                raise RuntimeError(f"game ended; see {self.run.folder / 'stdout.log'}")
            self.run.log.write(line)
            if line.startswith("{"):
                return json.loads(line)
        raise RuntimeError("no control reply in time")

    def send(self, command):
        self.run.proc.stdin.write(command + "\n")
        self.run.proc.stdin.flush()
        reply = self._reply()
        if reply.get("k") == "error":
            raise RuntimeError(f"control `{command}`: {reply}")
        return reply

    def quit(self):
        try:
            self.run.proc.stdin.write("quit\n")
            self.run.proc.stdin.close()
        except OSError:
            pass


def jobs_map(fn, items, jobs):
    """`fn` over `items` with up to `jobs` at once, results in input order."""
    if jobs <= 1:
        return [fn(i) for i in items]
    from concurrent.futures import ThreadPoolExecutor
    with ThreadPoolExecutor(jobs) as pool:
        return list(pool.map(fn, items))


def write_json(path, value):
    Path(path).write_text(json.dumps(value, indent=1) + "\n", encoding="utf-8")


def fail(message):
    print(f"playcheck: {message}", file=sys.stderr)
    return 2
