"""Ordinary test CLI: deliberately imports no MAS SDK."""
import argparse
import json
import os
import sys
import time
import subprocess

REVISION = 1
parser = argparse.ArgumentParser()
parser.add_argument("--service", required=True)
parser.add_argument("--invocation", required=True)
args = parser.parse_args()
task = json.load(sys.stdin)
output = {"service": args.service, "n": task.get("n", 0), "revision": REVISION,
          "declared": os.getenv("MAS_COMMAND_DECLARED"),
          "ambient": os.getenv("MAS_COMMAND_UNDECLARED"),
          "channel": os.getenv("MAS_WORKER_CHANNEL_FD"), "cwd": os.getcwd()}
if "receipt" in task:
    with open(task["receipt"], "x") as stream:
        json.dump({"invocation_id": args.invocation, "output": output}, stream)
        stream.flush()
        os.fsync(stream.fileno())
    descriptor = os.open(os.path.dirname(task["receipt"]), os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)
mode = task.get("mode")
if mode == "crash":
    sys.exit(7)
if mode == "sleep":
    child = subprocess.Popen(["/bin/sleep", "30"])
    if "child_pid" in task:
        with open(task["child_pid"], "w") as stream:
            stream.write(str(child.pid))
    time.sleep(30)
if mode == "overflow":
    print("x" * 1024)
elif mode == "float":
    print('{"value": 1.5}')
elif mode == "duplicate":
    print('{"value": 1, "value": 2}')
elif mode == "text":
    print("plain output")
else:
    print(json.dumps(output))
