"""
SOMA Body — The hardware/environment interface.

This is the fixed dispatcher that maps opcodes to real OS operations.
It is NOT generated, NOT neural, NOT interpreted. It is the body's
nervous system — a fixed wiring from opcode signals to muscle movements.

The Body also provides proprioception: self-knowledge of capabilities.
"""

import os
import platform
import shutil
import subprocess
from dataclasses import dataclass, field
from datetime import datetime


@dataclass
class ParamSlot:
    name: str       # e.g., "path", "content"
    type: str       # "path", "string"
    required: bool = True


@dataclass
class Operation:
    name: str
    opcode: int
    description: str
    params: list[ParamSlot] = field(default_factory=list)


# --- The Body Manifest ---
# This is the SOMA's proprioceptive knowledge of what its body can do.

OPERATIONS = [
    Operation("LIST_DIR", 0, "List files in a directory",
              [ParamSlot("path", "path")]),
    Operation("CREATE_FILE", 1, "Create a file with optional content",
              [ParamSlot("path", "path"), ParamSlot("content", "string", required=False)]),
    Operation("READ_FILE", 2, "Read contents of a file",
              [ParamSlot("path", "path")]),
    Operation("DELETE_FILE", 3, "Delete a file",
              [ParamSlot("path", "path")]),
    Operation("MAKE_DIR", 4, "Create a directory",
              [ParamSlot("path", "path")]),
    Operation("FILE_INFO", 5, "Get file metadata",
              [ParamSlot("path", "path")]),
    Operation("CURRENT_DIR", 6, "Get current working directory", []),
    Operation("SYSTEM_INFO", 7, "Get system information", []),
    Operation("CURRENT_TIME", 8, "Get current date and time", []),
    Operation("DISK_USAGE", 9, "Get disk usage statistics", []),
    Operation("PROCESS_LIST", 10, "List running processes", []),
    Operation("MOVE_FILE", 11, "Move or rename a file",
              [ParamSlot("source", "path"), ParamSlot("destination", "path")]),
    Operation("COPY_FILE", 12, "Copy a file",
              [ParamSlot("source", "path"), ParamSlot("destination", "path")]),
    Operation("FIND_FILE", 13, "Find files by name pattern",
              [ParamSlot("pattern", "string")]),
    Operation("FILE_EXISTS", 14, "Check if a file exists",
              [ParamSlot("path", "path")]),
]

NUM_OPERATIONS = len(OPERATIONS)
MAX_PARAM_SLOTS = 2


class Body:
    """The fixed nervous system. Maps (opcode, params) to OS execution."""

    def __init__(self):
        self.operations = {op.opcode: op for op in OPERATIONS}
        self._dispatch_table = {
            0:  self._list_dir,
            1:  self._create_file,
            2:  self._read_file,
            3:  self._delete_file,
            4:  self._make_dir,
            5:  self._file_info,
            6:  self._current_dir,
            7:  self._system_info,
            8:  self._current_time,
            9:  self._disk_usage,
            10: self._process_list,
            11: self._move_file,
            12: self._copy_file,
            13: self._find_file,
            14: self._file_exists,
        }

    def dispatch(self, opcode: int, params: list[str | None]) -> dict:
        """Execute an operation. Returns {success, result, error}."""
        op = self.operations.get(opcode)
        if op is None:
            return {"success": False, "result": None, "error": f"Unknown opcode: {opcode}"}

        # Filter to non-None params
        provided = [p for p in params if p is not None]
        required_count = sum(1 for p in op.params if p.required)

        if len(provided) < required_count:
            return {"success": False, "result": None,
                    "error": f"{op.name} requires {required_count} params, got {len(provided)}"}

        try:
            result = self._dispatch_table[opcode](*provided[:len(op.params)])
            return {"success": True, "result": result, "error": None}
        except Exception as e:
            return {"success": False, "result": None, "error": str(e)}

    def capabilities(self) -> list[dict]:
        """Proprioception: report what this body can do."""
        return [
            {
                "name": op.name,
                "opcode": op.opcode,
                "description": op.description,
                "params": [{"name": p.name, "type": p.type} for p in op.params],
            }
            for op in OPERATIONS
        ]

    # --- Operation implementations (the muscles) ---

    def _list_dir(self, path: str) -> list[str]:
        return sorted(os.listdir(os.path.expanduser(path)))

    def _create_file(self, path: str, content: str = "") -> str:
        path = os.path.expanduser(path)
        with open(path, "w") as f:
            f.write(content)
        return f"Created {path}"

    def _read_file(self, path: str) -> str:
        with open(os.path.expanduser(path), "r") as f:
            return f.read()

    def _delete_file(self, path: str) -> str:
        path = os.path.expanduser(path)
        os.remove(path)
        return f"Deleted {path}"

    def _make_dir(self, path: str) -> str:
        path = os.path.expanduser(path)
        os.makedirs(path, exist_ok=True)
        return f"Created directory {path}"

    def _file_info(self, path: str) -> dict:
        stat = os.stat(os.path.expanduser(path))
        return {
            "size_bytes": stat.st_size,
            "modified": datetime.fromtimestamp(stat.st_mtime).isoformat(),
            "mode": oct(stat.st_mode),
        }

    def _current_dir(self) -> str:
        return os.getcwd()

    def _system_info(self) -> dict:
        return {
            "system": platform.system(),
            "machine": platform.machine(),
            "processor": platform.processor(),
            "version": platform.version(),
        }

    def _current_time(self) -> str:
        return datetime.now().isoformat()

    def _disk_usage(self) -> dict:
        usage = shutil.disk_usage("/")
        return {
            "total_gb": round(usage.total / (1024**3), 2),
            "used_gb": round(usage.used / (1024**3), 2),
            "free_gb": round(usage.free / (1024**3), 2),
        }

    def _process_list(self) -> list[str]:
        result = subprocess.run(["ps", "aux"], capture_output=True, text=True)
        lines = result.stdout.strip().split("\n")
        return lines[:20]

    def _move_file(self, source: str, dest: str) -> str:
        shutil.move(os.path.expanduser(source), os.path.expanduser(dest))
        return f"Moved {source} -> {dest}"

    def _copy_file(self, source: str, dest: str) -> str:
        shutil.copy2(os.path.expanduser(source), os.path.expanduser(dest))
        return f"Copied {source} -> {dest}"

    def _find_file(self, pattern: str) -> list[str]:
        import glob as g
        results = g.glob(f"**/{pattern}", recursive=True)
        return results[:20]

    def _file_exists(self, path: str) -> bool:
        return os.path.exists(os.path.expanduser(path))
