"""Drive the production pause regression with X11 clicks (Linux/Xvfb).

Build: cargo build --locked -p prpr --example rpe_next_pause_regression
Run: xvfb-run -a python3 scripts/tests/run_next_pause_regression.py
Requires an available audio output or ALSA null configuration, X11 and XTest.
"""
import ctypes
import subprocess
import sys
import time


def main():
    x11 = ctypes.CDLL("libX11.so.6")
    xtst = ctypes.CDLL("libXtst.so.6")
    x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
    x11.XOpenDisplay.restype = ctypes.c_void_p
    x11.XFlush.argtypes = [ctypes.c_void_p]
    x11.XCloseDisplay.argtypes = [ctypes.c_void_p]
    xtst.XTestFakeMotionEvent.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_ulong]
    xtst.XTestFakeButtonEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
    display = x11.XOpenDisplay(None)
    if not display:
        raise RuntimeError("No X11 display; run under Xvfb")
    binary = sys.argv[1] if len(sys.argv) > 1 else "target/debug/examples/rpe_next_pause_regression"
    child = subprocess.Popen([binary], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
    try:
        for line in child.stdout:
            print(line, end="", flush=True)
            if not line.startswith("CLICK "):
                continue
            _, x, y, count = line.split()
            xtst.XTestFakeMotionEvent(display, -1, round(float(x)), round(float(y)), 0)
            for _ in range(int(count)):
                xtst.XTestFakeButtonEvent(display, 1, 1, 0)
                x11.XFlush(display)
                time.sleep(0.06)
                xtst.XTestFakeButtonEvent(display, 1, 0, 0)
                x11.XFlush(display)
                time.sleep(0.06)
        raise SystemExit(child.wait())
    finally:
        if child.poll() is None:
            child.terminate()
            child.wait()
        x11.XCloseDisplay(display)


if __name__ == "__main__":
    main()
