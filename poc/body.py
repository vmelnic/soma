"""
SOMA Body — Phase 2: Thin Primitive Executor.

The body is a dumb pipe to the OS. One line per primitive. Zero logic.
All sequencing, composition, and dependency management lives in the Mind.

19 primitives form the "instruction set" of this SOMA's body.
The Mind generates programs (sequences of primitives with data dependencies).
The Body blindly executes them step by step.
"""

import os
import platform
import shutil
import subprocess
from dataclasses import dataclass
from datetime import datetime
from enum import IntEnum


class Prim(IntEnum):
    """The primitive instruction set."""
    FILE_OPEN_R = 0
    FILE_OPEN_W = 1
    FILE_READ = 2
    FILE_WRITE = 3
    FILE_CLOSE = 4
    DIR_LIST = 5
    FILE_DELETE = 6
    FILE_STAT = 7
    DIR_CREATE = 8
    FILE_RENAME = 9
    FILE_COPY = 10
    FILE_EXISTS = 11
    SYS_CWD = 12
    SYS_INFO = 13
    SYS_TIME = 14
    SYS_DISK = 15
    SYS_PROCS = 16
    EMIT = 17
    STOP = 18


NUM_PRIMITIVES = len(Prim)
MAX_PROGRAM_STEPS = 8
START_TOKEN = NUM_PRIMITIVES  # 19, used as decoder start signal

OPCODE_NAMES = [p.name for p in Prim]

# Argument schema per opcode: (arg0_type, arg1_type)
# "none" = no argument, "span" = extract from input, "ref" = reference previous step
OPCODE_SCHEMA = {
    Prim.FILE_OPEN_R: ("span", "none"),
    Prim.FILE_OPEN_W: ("span", "none"),
    Prim.FILE_READ:   ("ref",  "none"),
    Prim.FILE_WRITE:  ("ref",  "span"),
    Prim.FILE_CLOSE:  ("ref",  "none"),
    Prim.DIR_LIST:    ("span", "none"),
    Prim.FILE_DELETE: ("span", "none"),
    Prim.FILE_STAT:   ("span", "none"),
    Prim.DIR_CREATE:  ("span", "none"),
    Prim.FILE_RENAME: ("span", "span"),
    Prim.FILE_COPY:   ("span", "span"),
    Prim.FILE_EXISTS: ("span", "none"),
    Prim.SYS_CWD:    ("none", "none"),
    Prim.SYS_INFO:   ("none", "none"),
    Prim.SYS_TIME:   ("none", "none"),
    Prim.SYS_DISK:   ("none", "none"),
    Prim.SYS_PROCS:  ("none", "none"),
    Prim.EMIT:       ("ref",  "none"),
    Prim.STOP:       ("none", "none"),
}


@dataclass
class ProgramStep:
    opcode: int
    arg0_type: str   # "none", "span", "ref"
    arg0_value: object  # str for span, int for ref, None for none
    arg1_type: str
    arg1_value: object

    def format(self, step_idx: int) -> str:
        """Format step for display."""
        name = OPCODE_NAMES[self.opcode]
        args = []
        for atype, aval in [(self.arg0_type, self.arg0_value),
                            (self.arg1_type, self.arg1_value)]:
            if atype == "span":
                args.append(f'"{aval}"')
            elif atype == "ref":
                args.append(f"${aval}")
            # none: skip
        args_str = ", ".join(args)
        if self.opcode == Prim.STOP:
            return "STOP"
        return f"${step_idx} = {name}({args_str})"


class ThinBody:
    """Thin executor. One OS call per primitive. Zero logic."""

    def execute_primitive(self, opcode: int, args: list) -> object:
        """Execute a single primitive. One line each. No logic."""
        match opcode:
            case Prim.FILE_OPEN_R: return open(os.path.expanduser(args[0]), "r")
            case Prim.FILE_OPEN_W: return open(os.path.expanduser(args[0]), "w")
            case Prim.FILE_READ:   return args[0].read()
            case Prim.FILE_WRITE:  args[0].write(args[1]); return True
            case Prim.FILE_CLOSE:  args[0].close(); return True
            case Prim.DIR_LIST:    return sorted(os.listdir(os.path.expanduser(args[0])))
            case Prim.FILE_DELETE: os.remove(os.path.expanduser(args[0])); return True
            case Prim.FILE_STAT:   s = os.stat(os.path.expanduser(args[0])); return {"size": s.st_size, "modified": datetime.fromtimestamp(s.st_mtime).isoformat()}
            case Prim.DIR_CREATE:  os.makedirs(os.path.expanduser(args[0]), exist_ok=True); return True
            case Prim.FILE_RENAME: shutil.move(os.path.expanduser(args[0]), os.path.expanduser(args[1])); return True
            case Prim.FILE_COPY:   shutil.copy2(os.path.expanduser(args[0]), os.path.expanduser(args[1])); return True
            case Prim.FILE_EXISTS: return os.path.exists(os.path.expanduser(args[0]))
            case Prim.SYS_CWD:    return os.getcwd()
            case Prim.SYS_INFO:   return {"system": platform.system(), "machine": platform.machine(), "processor": platform.processor()}
            case Prim.SYS_TIME:   return datetime.now().isoformat()
            case Prim.SYS_DISK:   u = shutil.disk_usage("/"); return {"total_gb": round(u.total / 1e9, 2), "used_gb": round(u.used / 1e9, 2), "free_gb": round(u.free / 1e9, 2)}
            case Prim.SYS_PROCS:  return subprocess.run(["ps", "aux"], capture_output=True, text=True).stdout.strip().split("\n")[:15]
            case Prim.EMIT:       return args[0]  # pass through for display
            case Prim.STOP:       return None

    def execute_program(self, steps: list[ProgramStep]) -> dict:
        """Execute a program. Resolve refs, call primitives, collect results."""
        results = []
        output = None
        trace = []

        for i, step in enumerate(steps):
            if step.opcode == Prim.STOP:
                trace.append({"step": i, "op": "STOP"})
                break

            # Resolve arguments
            resolved = []
            for atype, aval in [(step.arg0_type, step.arg0_value),
                                (step.arg1_type, step.arg1_value)]:
                if atype == "ref":
                    if aval < len(results):
                        resolved.append(results[aval])
                    else:
                        return {"success": False, "output": None,
                                "trace": trace, "error": f"Invalid ref ${aval} at step {i}"}
                elif atype == "span":
                    resolved.append(aval)
                # none: don't append

            try:
                result = self.execute_primitive(step.opcode, resolved)
                results.append(result)
                trace.append({"step": i, "op": OPCODE_NAMES[step.opcode], "result_type": type(result).__name__})

                if step.opcode == Prim.EMIT:
                    output = result

            except Exception as e:
                # Close any open file handles before returning
                for r in results:
                    if hasattr(r, "close") and not getattr(r, "closed", True):
                        try:
                            r.close()
                        except Exception:
                            pass
                return {"success": False, "output": None, "trace": trace, "error": str(e)}

        return {"success": True, "output": output, "trace": trace, "error": None}

    def capabilities(self) -> list[dict]:
        """Proprioception: report available primitives."""
        return [
            {"opcode": p.value, "name": p.name,
             "args": OPCODE_SCHEMA[p]}
            for p in Prim
        ]
