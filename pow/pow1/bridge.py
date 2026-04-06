"""
SOMA Generic Execution Bridge — POW 1 (fully data-driven).

Dispatches on CALL PATTERNS, not function names.
6 patterns (like CPU addressing modes). Each is generic.
No function name appears in the execution path.

Patterns:
  direct       — marshal args, call, return result
  buffered_read — alloc buffer, pass as arg, decode content
  write_bytes  — data arg with computed length
  struct_query — alloc struct, pass by ref, extract fields
  iterate      — loop until NULL, collect d_name entries
  buffered_str — alloc string buffer, decode return
"""

import ctypes
import os
from datetime import datetime, timezone

from pow.pow1.discovery import CallingConvention, EMIT_ID, STOP_ID


class ProgramStep:
    __slots__ = ("conv_id", "arg_types", "arg_values")

    def __init__(self, conv_id, arg_types, arg_values):
        self.conv_id = conv_id
        self.arg_types = arg_types
        self.arg_values = arg_values

    def format(self, step_idx, catalog):
        if self.conv_id == STOP_ID:
            return "STOP"
        if self.conv_id == EMIT_ID:
            ref = self.arg_values[0] if self.arg_values else "?"
            return f"${step_idx} = EMIT(${ref})"
        conv = catalog[self.conv_id]
        args = []
        for at, av in zip(self.arg_types, self.arg_values):
            if at == "span":
                args.append(f'"{av}"')
            elif at == "ref":
                args.append(f"${av}")
        return f"${step_idx} = libc.{conv.function}({', '.join(args)})"


def _to_bytes(s):
    return s.encode("utf-8") if isinstance(s, str) else s


# ============================================================================
# Transform functions for struct field extraction
# ============================================================================

_TRANSFORMS = {
    "raw": lambda v: v,
    "decode": lambda v: v.decode("utf-8", errors="replace") if isinstance(v, bytes) else str(v),
    "oct": lambda v: oct(v),
    "timestamp": lambda v: datetime.fromtimestamp(v.tv_sec).isoformat(),
    "isotime": lambda v: datetime.fromtimestamp(v, tz=timezone.utc).isoformat(),
}


# ============================================================================
# The Generic Bridge — 6 patterns, zero function name dispatch
# ============================================================================

class GenericBridge:

    def __init__(self, catalog, libc):
        self.catalog = catalog
        self.libc = libc
        self._funcs = {}
        for conv in catalog:
            if conv.function not in self._funcs:
                self._funcs[conv.function] = getattr(libc, conv.function)

    def _call_convention(self, conv, resolved):
        """Execute a calling convention by its pattern. Fully data-driven.
        No function name appears here — only pattern + catalog data."""
        func = self._funcs[conv.function]
        func.argtypes = conv.c_argtypes
        func.restype = conv.c_restype
        pattern = conv.call_pattern

        # Marshal variable args (string->bytes, others pass through)
        c_args = []
        for a_spec, val in zip(conv.var_args, resolved):
            if a_spec["type"] == "string":
                c_args.append(_to_bytes(val))
            else:
                c_args.append(val)

        if pattern == "direct":
            # Append fixed args from catalog data
            for key in ("flags", "mode"):
                if key in conv.fixed_args:
                    c_args.append(conv.fixed_args[key])
            result = func(*c_args)
            return (result == 0) if conv.result_eq_zero else result

        if pattern == "buffered_read":
            buf = ctypes.create_string_buffer(conv.buffer_size)
            n = func(c_args[0], buf, conv.buffer_size)
            return buf.raw[:max(n, 0)].decode("utf-8", errors="replace") if n > 0 else ""

        if pattern == "write_bytes":
            data = _to_bytes(resolved[1])
            return func(c_args[0], data, len(data))

        if pattern == "struct_query":
            struct_buf = conv.struct_class()
            call_args = [_to_bytes(a) if isinstance(a, str) else a for a in c_args]
            call_args.append(ctypes.byref(struct_buf))
            if conv.null_arg:
                call_args.append(None)
            # Reorder: struct pointer must be first for no-vararg calls (uname),
            # or after path for stat. The c_argtypes order determines this.
            # Since we set argtypes correctly, just build args in order:
            if not c_args:
                # No variable args (uname, gettimeofday) — struct ptr is first
                ordered = [ctypes.byref(struct_buf)]
                if conv.null_arg:
                    ordered.append(None)
            else:
                # Variable args first, then struct ptr (stat)
                ordered = [_to_bytes(a) if isinstance(a, str) else a for a in c_args]
                ordered.append(ctypes.byref(struct_buf))
            func(*ordered)
            # Extract fields from struct using catalog-defined field map
            result = {}
            for out_key, field_name, transform in conv.struct_fields:
                val = getattr(struct_buf, field_name)
                result[out_key] = _TRANSFORMS[transform](val)
            return result

        if pattern == "iterate":
            func.restype = ctypes.POINTER(conv.struct_class)
            entries = []
            while True:
                ptr = func(*c_args)
                if not ptr:
                    break
                name = ptr.contents.d_name.decode("utf-8", errors="replace")
                if name not in (".", ".."):
                    entries.append(name)
            return sorted(entries)

        if pattern == "buffered_str":
            buf = ctypes.create_string_buffer(conv.buffer_size)
            result = func(buf, conv.buffer_size)
            return result.decode("utf-8") if result else None

        return None

    def execute_program(self, steps, on_step=None):
        results = []
        output = None
        trace = []

        for i, step in enumerate(steps):
            if step.conv_id == STOP_ID:
                trace.append({"step": i, "op": "STOP", "ok": True})
                if on_step:
                    on_step(i, "STOP", "")
                break

            if step.conv_id == EMIT_ID:
                ref_idx = step.arg_values[0] if step.arg_values else 0
                if isinstance(ref_idx, int) and ref_idx < len(results):
                    output = results[ref_idx]
                trace.append({"step": i, "op": "EMIT", "ok": True})
                if on_step:
                    on_step(i, "EMIT", "")
                continue

            conv = self.catalog[step.conv_id]

            resolved = []
            for at, av in zip(step.arg_types, step.arg_values):
                if at == "ref":
                    if isinstance(av, int) and av < len(results):
                        resolved.append(results[av])
                    else:
                        self._cleanup(results)
                        return {"success": False, "output": None, "trace": trace,
                                "error": f"Step {i} (libc.{conv.function}): invalid ref ${av}"}
                elif at == "span":
                    resolved.append(os.path.expanduser(av) if av else "")

            try:
                result = self._call_convention(conv, resolved)
                results.append(result)
                summary = self._summarize(result)
                trace.append({"step": i, "op": f"libc.{conv.function}", "ok": True,
                              "summary": summary})
                if on_step:
                    on_step(i, f"libc.{conv.function}", summary)
            except Exception as e:
                if on_step:
                    on_step(i, f"libc.{conv.function}", f"ERROR: {e}")
                self._cleanup(results)
                return {"success": False, "output": None, "trace": trace,
                        "error": f"Step {i} (libc.{conv.function}): {e}"}

        return {"success": True, "output": output, "trace": trace, "error": None}

    def _cleanup(self, results):
        close_func = self._funcs.get("close")
        if close_func:
            for r in results:
                if isinstance(r, int) and r > 2:
                    try:
                        close_func.argtypes = [ctypes.c_int]
                        close_func(r)
                    except Exception:
                        pass

    def _summarize(self, result):
        if result is None: return ""
        if isinstance(result, bool): return "yes" if result else "no"
        if isinstance(result, int): return f"fd={result}" if result > 2 else str(result)
        if isinstance(result, str): return f"{len(result)} chars" if len(result) > 50 else result
        if isinstance(result, list): return f"{len(result)} entries"
        if isinstance(result, dict): return ", ".join(f"{k}={v}" for k, v in list(result.items())[:3])
        return type(result).__name__
