#!/usr/bin/env python3
"""Reorder and stub stdlib.wasm: move WASI imports first, replace nexus:cli with stubs.

This eliminates nexus:cli imports entirely from stdlib.wasm so the wasm_merge
can use identity remaps and skip dep code rewriting.

Output: stdlib.wasm with only WASI imports + stub functions for former nexus:cli.
"""
import sys

def read_uleb128(data, pos):
    val, shift = 0, 0
    while True:
        b = data[pos]; pos += 1
        val |= (b & 0x7f) << shift
        if b < 0x80: return val, pos
        shift += 7

def read_sleb128(data, pos):
    val, shift = 0, 0
    while True:
        b = data[pos]; pos += 1
        val |= (b & 0x7f) << shift
        shift += 7
        if b < 0x80:
            if shift < 64 and (b & 0x40): val -= (1 << shift)
            return val, pos

def write_uleb128(val):
    out = bytearray()
    while True:
        b = val & 0x7f; val >>= 7
        if val: out.append(b | 0x80)
        else: out.append(b); break
    return bytes(out)

def parse_sections(data):
    sections = []
    pos = 8
    while pos < len(data):
        sec_id = data[pos]; pos += 1
        sec_size, pos = read_uleb128(data, pos)
        sections.append((sec_id, pos, sec_size))
        pos += sec_size
    return sections

def parse_imports(data, sec_offset):
    pos = sec_offset
    count, pos = read_uleb128(data, pos)
    imports = []
    for _ in range(count):
        start = pos
        mod_len, pos = read_uleb128(data, pos)
        mod_name = data[pos:pos+mod_len].decode('utf-8'); pos += mod_len
        name_len, pos = read_uleb128(data, pos)
        pos += name_len
        kind = data[pos]; pos += 1
        func_type = None
        if kind == 0:
            func_type, pos = read_uleb128(data, pos)
        elif kind == 1:
            pos += 1; _, pos = read_uleb128(data, pos); flags = data[pos-1]
            if _ & 1: _, pos = read_uleb128(data, pos)
        elif kind == 2:
            flags, pos = read_uleb128(data, pos)
            _, pos = read_uleb128(data, pos)
            if flags & 1: _, pos = read_uleb128(data, pos)
        elif kind == 3:
            pos += 2
        raw = bytes(data[start:pos])
        imports.append((mod_name, kind, func_type, raw))
    return imports

def rewrite_body(data, start, end, func_remap, num_old_imports):
    """Rewrite function body, remapping call/ref.func targets. Copy all other bytes verbatim."""
    out = bytearray()
    pos = start
    # Copy locals
    num_locals, new_pos = read_uleb128(data, pos)
    out.extend(data[pos:new_pos]); pos = new_pos
    for _ in range(num_locals):
        _, new_pos = read_uleb128(data, pos); new_pos += 1
        out.extend(data[pos:new_pos]); pos = new_pos
    # Instructions
    while pos < end:
        op = data[pos]
        if op in (0x10, 0x12):  # call, return_call
            out.append(op); pos += 1
            old_pos = pos; idx, pos = read_uleb128(data, old_pos)
            new_idx = func_remap.get(idx, idx)
            if new_idx != idx: out.extend(write_uleb128(new_idx))
            else: out.extend(data[old_pos:pos])
        elif op == 0xD2:  # ref.func
            out.append(op); pos += 1
            old_pos = pos; idx, pos = read_uleb128(data, old_pos)
            new_idx = func_remap.get(idx, idx)
            if new_idx != idx: out.extend(write_uleb128(new_idx))
            else: out.extend(data[old_pos:pos])
        else:
            out.append(op); pos += 1
            if op in (0x02, 0x03, 0x04):
                b = data[pos]
                if b == 0x40 or b >= 0x60: out.append(data[pos]); pos += 1
                else: _, np = read_sleb128(data, pos); out.extend(data[pos:np]); pos = np
            elif op == 0x06:
                b = data[pos]
                if b == 0x40 or b >= 0x60: out.append(data[pos]); pos += 1
                else: _, np = read_sleb128(data, pos); out.extend(data[pos:np]); pos = np
                nh, np = read_uleb128(data, pos); out.extend(data[pos:np]); pos = np
                for _ in range(nh):
                    _, np = read_uleb128(data, pos); out.extend(data[pos:np]); pos = np
                    _, np = read_uleb128(data, pos); out.extend(data[pos:np]); pos = np
            elif op in (0x08, 0x0C, 0x0D, 0x20, 0x21, 0x22, 0x23, 0x24):
                _, np = read_uleb128(data, pos); out.extend(data[pos:np]); pos = np
            elif op == 0x0E:
                cnt, np = read_uleb128(data, pos); out.extend(data[pos:np]); pos = np
                for _ in range(cnt + 1):
                    _, np = read_uleb128(data, pos); out.extend(data[pos:np]); pos = np
            elif op == 0x11:
                _, np = read_uleb128(data, pos); out.extend(data[pos:np]); pos = np
                _, np = read_uleb128(data, pos); out.extend(data[pos:np]); pos = np
            elif 0x28 <= op <= 0x3E:
                _, np = read_uleb128(data, pos); out.extend(data[pos:np]); pos = np
                _, np = read_uleb128(data, pos); out.extend(data[pos:np]); pos = np
            elif op in (0x3F, 0x40): out.append(data[pos]); pos += 1
            elif op == 0x41: _, np = read_sleb128(data, pos); out.extend(data[pos:np]); pos = np
            elif op == 0x42: _, np = read_sleb128(data, pos); out.extend(data[pos:np]); pos = np
            elif op == 0x43: out.extend(data[pos:pos+4]); pos += 4
            elif op == 0x44: out.extend(data[pos:pos+8]); pos += 8
            elif op == 0xFC:
                sub_op, np = read_uleb128(data, pos); out.extend(data[pos:np]); pos = np
                if sub_op <= 7: pass
                elif sub_op == 10: out.extend(data[pos:pos+2]); pos += 2
                elif sub_op == 11: out.append(data[pos]); pos += 1
                elif sub_op in (8, 9):
                    _, np = read_uleb128(data, pos); out.extend(data[pos:np]); pos = np
                    if sub_op == 8: out.append(data[pos]); pos += 1
                elif 12 <= sub_op <= 17:
                    _, np = read_uleb128(data, pos); out.extend(data[pos:np]); pos = np
                    if sub_op in (12, 14):
                        _, np = read_uleb128(data, pos); out.extend(data[pos:np]); pos = np
    return bytes(out)

def main():
    if len(sys.argv) != 2:
        print(f"Usage: {sys.argv[0]} <stdlib.wasm>", file=sys.stderr); sys.exit(1)

    path = sys.argv[1]
    with open(path, 'rb') as f: data = bytearray(f.read())

    sections = parse_sections(data)
    sec_map = {s[0]: (s[1], s[2]) for s in sections}

    imports = parse_imports(data, sec_map[2][0])
    wasi_idxs = [i for i, imp in enumerate(imports) if imp[0].startswith('wasi_snapshot')]
    cli_idxs = [i for i, imp in enumerate(imports) if imp[0].startswith('nexus:cli')]

    if not cli_idxs:
        print("No nexus:cli imports — nothing to do"); return

    # Count func imports
    num_wasi_func = sum(1 for i in wasi_idxs if imports[i][1] == 0)
    num_cli_func = sum(1 for i in cli_idxs if imports[i][1] == 0)
    num_old_func_imports = sum(1 for imp in imports if imp[1] == 0)
    num_new_func_imports = num_wasi_func  # only WASI func imports remain

    # Stub functions replace cli func imports. They go right after the new imports.
    # New layout: [0..num_wasi_func) = WASI imports, [num_wasi_func..num_wasi_func+num_cli_func) = stubs, [num_wasi_func+num_cli_func..) = original locals

    # Build function index remap: old_func_idx -> new_func_idx
    func_remap = {}
    old_fidx = 0
    wasi_fidx = 0
    cli_fidx = 0
    for i, imp in enumerate(imports):
        if imp[1] != 0: continue  # skip non-function imports
        if imp[0].startswith('wasi_snapshot'):
            func_remap[old_fidx] = wasi_fidx
            wasi_fidx += 1
        else:  # nexus:cli
            func_remap[old_fidx] = num_wasi_func + cli_fidx  # stub position
            cli_fidx += 1
        old_fidx += 1

    # Local functions shift: old local N was at old_func_idx = num_old_func_imports + N
    # New local N is at num_wasi_func + num_cli_func + N
    local_shift = (num_wasi_func + num_cli_func) - num_old_func_imports
    # Add local remap entries (for element section, exports, etc.)
    # We handle locals by shifting in the remap lookup

    print(f"Stubbing: {num_wasi_func} WASI + {num_cli_func} CLI func imports")
    print(f"Local function shift: {local_shift}")

    # Get cli func type indices for stubs
    cli_func_types = []
    for i in cli_idxs:
        if imports[i][1] == 0:
            cli_func_types.append(imports[i][2])

    # --- Rebuild import section (WASI only) ---
    new_imp = bytearray()
    wasi_count = len(wasi_idxs)
    new_imp.extend(write_uleb128(wasi_count))
    for i in wasi_idxs:
        new_imp.extend(imports[i][3])

    # --- Rebuild function section: prepend stubs ---
    func_off, func_size = sec_map[3]
    func_count, fpos = read_uleb128(data, func_off)
    new_func = bytearray()
    new_func.extend(write_uleb128(num_cli_func + func_count))
    # Stub type indices
    for t in cli_func_types:
        new_func.extend(write_uleb128(t))
    # Original function types (verbatim)
    new_func.extend(data[fpos:func_off + func_size])

    # --- Rebuild code section: prepend stub bodies ---
    code_off, code_size = sec_map[10]
    code_count, cpos = read_uleb128(data, code_off)
    new_code = bytearray()
    new_code.extend(write_uleb128(num_cli_func + code_count))
    # Stub bodies: unreachable + end
    for _ in range(num_cli_func):
        new_code.extend(write_uleb128(3))  # body size = 3
        new_code.append(0x00)  # 0 locals
        new_code.append(0x00)  # unreachable
        new_code.append(0x0B)  # end
    # Rewrite original code bodies (remap call targets)
    for _ in range(code_count):
        body_size, body_start = read_uleb128(data, cpos)
        body_end = body_start + body_size
        new_body = rewrite_body(data, body_start, body_end, func_remap, num_old_func_imports)
        new_code.extend(write_uleb128(len(new_body)))
        new_code.extend(new_body)
        cpos = body_end

    # --- Rebuild element section ---
    new_elem = None
    if 9 in sec_map:
        elem_off, elem_size = sec_map[9]
        pos = elem_off
        elem_count, pos = read_uleb128(data, pos)
        eb = bytearray()
        eb.extend(write_uleb128(elem_count))
        for _ in range(elem_count):
            kind = data[pos]; pos += 1; eb.append(kind)
            if kind == 0:
                while data[pos] != 0x0B: eb.append(data[pos]); pos += 1
                eb.append(data[pos]); pos += 1
                n, pos = read_uleb128(data, pos); eb.extend(write_uleb128(n))
                for _ in range(n):
                    idx, pos = read_uleb128(data, pos)
                    new_idx = func_remap.get(idx, idx + local_shift) if idx < num_old_func_imports else idx + local_shift
                    eb.extend(write_uleb128(new_idx))
        new_elem = bytes(eb)

    # --- Rebuild export section ---
    exp_off, exp_size = sec_map[7]
    pos = exp_off
    exp_count, pos = read_uleb128(data, pos)
    new_exp = bytearray()
    new_exp.extend(write_uleb128(exp_count))
    for _ in range(exp_count):
        nl, pos = read_uleb128(data, pos)
        new_exp.extend(write_uleb128(nl))
        new_exp.extend(data[pos:pos+nl]); pos += nl
        kind = data[pos]; pos += 1; new_exp.append(kind)
        idx, pos = read_uleb128(data, pos)
        if kind == 0:
            new_idx = func_remap.get(idx, idx + local_shift) if idx < num_old_func_imports else idx + local_shift
            new_exp.extend(write_uleb128(new_idx))
        else:
            new_exp.extend(write_uleb128(idx))

    # --- Assemble output ---
    output = bytearray(data[:8])
    for sec_id, sec_off, sec_size in sections:
        if sec_id == 2:
            output.append(sec_id); output.extend(write_uleb128(len(new_imp))); output.extend(new_imp)
        elif sec_id == 3:
            output.append(sec_id); output.extend(write_uleb128(len(new_func))); output.extend(new_func)
        elif sec_id == 7:
            output.append(sec_id); output.extend(write_uleb128(len(new_exp))); output.extend(new_exp)
        elif sec_id == 9 and new_elem:
            output.append(sec_id); output.extend(write_uleb128(len(new_elem))); output.extend(new_elem)
        elif sec_id == 10:
            output.append(sec_id); output.extend(write_uleb128(len(new_code))); output.extend(new_code)
        else:
            output.append(sec_id); output.extend(write_uleb128(sec_size)); output.extend(data[sec_off:sec_off+sec_size])

    out_path = path + '.stubbed'
    with open(out_path, 'wb') as f: f.write(output)
    print(f"Wrote {out_path} ({len(output)} bytes, was {len(data)})")

if __name__ == '__main__':
    main()
