#!/usr/bin/env python3
"""Resolve AgnostOS profile samples and render a self-contained flame graph."""

import html
import re
import shutil
import subprocess
import sys
from pathlib import Path


def fail(message):
    raise SystemExit(message)


def symbolize(pdb, addresses, anchor):
    tool = shutil.which("llvm-pdbutil")
    if not tool:
        fail("llvm-pdbutil is required to resolve UEFI kernel symbols")
    sections = subprocess.run(
        [tool, "dump", "-section-headers", str(pdb)],
        text=True, capture_output=True, check=True,
    ).stdout
    section_bases = {}
    current_section = 0
    for line in sections.splitlines():
        header = re.search(r"SECTION HEADER #(\d+)", line)
        if header:
            current_section = int(header.group(1))
        address = re.search(r"^\s*([0-9A-Fa-f]+) virtual address$", line)
        if address and current_section:
            section_bases[current_section] = int(address.group(1), 16)

    dump = subprocess.run(
        [tool, "dump", "-symbols", str(pdb)],
        text=True, capture_output=True, check=True,
    ).stdout.splitlines()
    symbols = []
    for index, line in enumerate(dump):
        match = re.search(r"S_(?:G|L)PROC32.*?`(.+?)`", line)
        if not match:
            continue
        for detail in dump[index + 1:index + 4]:
            address = re.search(
                r"addr = ([0-9A-Fa-f]+):([0-9A-Fa-f]+), code size = (\d+)", detail
            )
            if address:
                section = int(address.group(1), 16)
                if section in section_bases:
                    # llvm-pdbutil prints section offsets and code sizes in
                    # decimal, even though section virtual addresses are hex.
                    start = section_bases[section] + int(address.group(2))
                    symbols.append((start, start + int(address.group(3)), match.group(1)))
                break
    symbols.sort()
    if not symbols:
        fail(f"no function symbols found in {pdb}")

    anchor_symbol = next(
        (symbol for symbol in symbols if symbol[2] == "agnostos::profiling::initialize"),
        None,
    )
    if not anchor_symbol:
        fail("profiling anchor function is missing from the PDB")
    image_base = anchor - anchor_symbol[0]

    result = {}
    cursor = 0
    for runtime_address in sorted(addresses):
        rva = runtime_address - image_base
        while cursor + 1 < len(symbols) and symbols[cursor + 1][0] <= rva:
            cursor += 1
        start, end, name = symbols[cursor]
        if start <= rva < end:
            result[runtime_address] = name
        elif rva >= end and rva - end <= 256:
            result[runtime_address] = f"{name}+0x{rva - start:x}"
        else:
            result[runtime_address] = None
    return result


def add_stack(root, names):
    node = root
    node["samples"] += 1
    for name in names:
        node = node["children"].setdefault(
            name, {"samples": 0, "children": {}}
        )
        node["samples"] += 1


def render(root, output):
    row_height = 24
    frame_height = 22
    max_depth = 0

    def depth(node, current=0):
        nonlocal max_depth
        max_depth = max(max_depth, current)
        for child in node["children"].values():
            depth(child, current + 1)

    depth(root)
    # Root is depth 0 and is not drawn; each descendant level is one row.
    rows = max(max_depth, 1)
    height = rows * row_height
    colors = ("#f2c14e", "#f78154", "#e85d75", "#c44569", "#f4a261")
    rects = []
    frame_index = []
    narrow_index = []
    clip_id = 0
    total = max(root["samples"], 1)

    def draw(node, x, y, w, label, path):
        nonlocal clip_id
        if label is not None:
            clip_id += 1
            this_frame = f"frame-{clip_id}"
            color = colors[(y // row_height) % len(colors)]
            shown = html.escape(label, quote=False)
            frame_index.append((this_frame, label, node["samples"]))
            # Hide labels until the full name fits the live pixel width.
            label_width = len(label) * 7 + 12
            title = html.escape(f"{label} — {node['samples']} samples")
            if w * 1400 < label_width:
                narrow_index.append((this_frame, label, node["samples"], color))
            rects.append(
                f'<button type="button" class="frame" id="{this_frame}" data-path="{path}" '
                f'data-x="{x:.8f}" data-width="{w:.8f}" data-samples="{node["samples"]}" '
                f'data-label-width="{label_width}" '
                f'style="left:{x * 100:.4f}%;width:{w * 100:.4f}%;top:{y}px;'
                f'height:{frame_height}px;background:{color}" title="{title}">'
                f'<span class="frame-label">{shown}</span></button>'
            )
        cursor = x
        for child_index, (child_name, child) in enumerate(sorted(node["children"].items())):
            child_width = w * child["samples"] / max(node["samples"], 1)
            child_path = f"{path}.{child_index}" if path else str(child_index)
            draw(child, cursor, y - row_height, child_width, child_name, child_path)
            cursor += child_width

    # Flame graphs place root frames at the bottom and callees above them.
    for root_index, (name, node) in enumerate(sorted(root["children"].items())):
        draw(node, 0, height - row_height, node["samples"] / total, name, str(root_index))

    index_items = "".join(
        f'<li><button type="button" data-target="{frame_id}">'
        f'<span class="function-name">{html.escape(name)}</span>'
        f'<span class="function-count">{count} samples · {count * 100 / total:.1f}%</span>'
        f'</button></li>'
        for frame_id, name, count in sorted(frame_index, key=lambda item: (-item[2], item[1]))
    )
    narrow_items = "".join(
        f'<li><button type="button" data-target="{frame_id}">'
        f'<span class="narrow-swatch" style="background:{color}"></span>'
        f'<span class="function-name">{html.escape(name)}</span>'
        f'<span class="function-count">{count} samples · zoom</span>'
        f'</button></li>'
        for frame_id, name, count, color in sorted(narrow_index, key=lambda item: (-item[2], item[1]))
    )

    document = f'''<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>AgnostOS Flame Graph</title>
<style>
:root{{--ink:#20242c;--muted:#667085;--line:#e4e7ec;--paper:#fff;--wash:#f4f6f8}}
*{{box-sizing:border-box}}
body{{margin:0;background:var(--wash);color:var(--ink);font:14px/1.45 system-ui,-apple-system,"Segoe UI",sans-serif}}
main{{max-width:1500px;margin:0 auto;padding:28px clamp(16px,3vw,40px) 48px}}
header{{display:flex;align-items:flex-start;justify-content:space-between;gap:20px;margin-bottom:18px}}
h1{{font-size:22px;line-height:1.2;margin:0 0 5px;letter-spacing:-.02em}}
.subtitle{{color:var(--muted);margin:0}}
.sample-count{{white-space:nowrap;border:1px solid var(--line);background:var(--paper);border-radius:999px;padding:7px 12px;font-weight:650}}
.card{{background:var(--paper);border:1px solid var(--line);border-radius:12px;box-shadow:0 1px 2px #1018280a}}
.chart-card{{padding:16px}}
.toolbar{{display:flex;align-items:center;justify-content:space-between;gap:12px;margin-bottom:12px}}
.hint{{color:var(--muted)}}button,input{{font:inherit}}button{{cursor:pointer}}
.reset{{border:1px solid #cfd4dc;background:#fff;color:var(--ink);border-radius:7px;padding:7px 11px;font-weight:600}}
.reset:hover{{background:#f8fafc;border-color:#98a2b3}}
.chart-wrap{{width:100%;overflow:hidden;border:1px solid #edf0f2;border-radius:8px;background:#fff}}
.chart{{position:relative;width:100%;height:{height}px;background:#fff}}
.frame{{position:absolute;box-sizing:border-box;appearance:none;-webkit-appearance:none;margin:0;padding:0 6px;border:1px solid #fff;border-radius:2px;color:var(--ink);font:11px/{frame_height}px ui-monospace,SFMono-Regular,Consolas,monospace;text-align:left;overflow:hidden;white-space:nowrap;cursor:pointer}}
.frame:hover,.frame:focus{{outline:1.5px solid #20242c;outline-offset:-1.5px;z-index:1}}
.frame-label{{display:block;overflow:hidden;text-overflow:ellipsis;pointer-events:none}}
#details{{min-height:42px;margin-top:12px;padding:10px 12px;border-radius:7px;background:#f8fafc;color:#344054;overflow-wrap:anywhere}}
.functions{{margin-top:16px;overflow:hidden}}.functions-head{{padding:16px 18px 12px;border-bottom:1px solid var(--line)}}
.functions-head h2{{font-size:16px;margin:0 0 10px}}
#frame-filter{{width:min(100%,460px);padding:9px 11px;border:1px solid #cfd4dc;border-radius:7px;outline:none}}
#frame-filter:focus{{border-color:#d97706;box-shadow:0 0 0 3px #fdb02233}}
#frame-index{{list-style:none;padding:0;margin:0;max-height:360px;overflow:auto}}
#frame-index li+li{{border-top:1px solid #f0f1f3}}
#frame-index button{{display:flex;width:100%;align-items:center;justify-content:space-between;gap:16px;text-align:left;border:0;background:#fff;padding:10px 18px;color:var(--ink)}}
#frame-index button:hover,#frame-index button:focus{{background:#fff8eb;outline:none}}
.function-name{{font-family:ui-monospace,SFMono-Regular,Consolas,monospace;overflow-wrap:anywhere}}
.function-count{{flex:none;color:var(--muted);font-size:12px}}
.narrow-calls{{margin-top:16px;overflow:hidden}}.narrow-head{{padding:14px 18px;border-bottom:1px solid var(--line)}}
.narrow-head h2{{font-size:15px;margin:0 0 3px}}.narrow-head p{{margin:0;color:var(--muted)}}
.narrow-calls ul{{list-style:none;padding:0;margin:0;max-height:240px;overflow:auto}}
.narrow-calls li+li{{border-top:1px solid #f0f1f3}}
.narrow-calls button{{display:flex;width:100%;align-items:center;gap:10px;text-align:left;border:0;background:#fff;padding:9px 18px;color:var(--ink)}}
.narrow-calls button:hover,.narrow-calls button:focus{{background:#fff8eb;outline:none}}
.narrow-swatch{{width:12px;height:12px;flex:none;border:1px solid #0002;border-radius:3px}}
#empty-state{{padding:14px 18px;color:var(--muted)}}
@media(max-width:640px){{main{{padding:18px 12px 32px}}header{{display:block}}.sample-count{{display:inline-block;margin-top:12px}}.chart-card{{padding:10px}}.toolbar{{align-items:flex-start;flex-direction:column}}.function-count{{font-size:11px}}}}
</style></head>
<body><main>
<header><div><h1>AgnostOS flame graph</h1><p class="subtitle">Sampled call stacks · select a frame to inspect its callees</p></div>
<div class="sample-count">{total:,} samples</div></header>
<section class="card chart-card" aria-label="Flame graph">
<div class="toolbar"><span class="hint" id="zoom-hint">Showing the full profile</span><button class="reset" id="reset-zoom" type="button">Reset view</button></div>
<div class="chart-wrap"><div class="chart" role="img" aria-label="Sampled call stack flame graph">{''.join(rects)}</div></div>
<div id="details" aria-live="polite">Hover over a frame for its function and sample count. Click to zoom.</div>
</section>
<section class="card narrow-calls" aria-label="Frames too narrow for labels">
<div class="narrow-head"><h2>Too narrow to label</h2><p>These frames are tiny because they have few samples. Select a function to zoom in.</p></div>
<ul>{narrow_items}</ul>
</section>
<section class="card functions" aria-label="Functions in profile">
<div class="functions-head"><h2>Functions <span class="hint">({len({name for _, name, _ in frame_index})})</span></h2>
<input id="frame-filter" type="search" placeholder="Search functions" aria-label="Search functions"></div>
<ul id="frame-index">{index_items}</ul><div id="empty-state" hidden>No matching functions.</div>
</section></main>
<script>
const frames=[...document.querySelectorAll('.frame')];
const details=document.getElementById('details');
const zoomHint=document.getElementById('zoom-hint');
const chart=document.querySelector('.chart');
function showDetails(frame){{details.textContent=frame.getAttribute('title');}}
function place(item,x,w){{
  item.style.left=(x*100)+'%';
  item.style.width=(w*100)+'%';
  const label=item.querySelector('.frame-label');
  if(label)label.style.visibility=(w*chart.clientWidth>=140)?'visible':'hidden';
}}
function zoom(frame){{
  const path=frame.dataset.path;
  for(const item of frames){{
    const visible=item.dataset.path===path||item.dataset.path.startsWith(path+'.');
    item.style.display=visible?'':'none';
  }}
  // A zoomed view answers “what called this function?” / “what does it call?”
  // without letting the selected function's self samples leave its callees
  // compressed into a few pixels. Rebase each sibling set to fill its parent.
  function layoutBranch(parent,x,width){{
    place(parent,x,width);
    const prefix=parent.dataset.path+'.';
    const children=frames.filter(item=>item.dataset.path.startsWith(prefix)&&
      !item.dataset.path.slice(prefix.length).includes('.'))
      .sort((a,b)=>Number(a.dataset.x)-Number(b.dataset.x));
    const childSamples=children.reduce((sum,item)=>sum+Number(item.dataset.samples),0);
    if(childSamples===0)return;
    let cursor=x;
    for(const child of children){{
      const childWidth=width*Number(child.dataset.samples)/childSamples;
      layoutBranch(child,cursor,childWidth);
      cursor+=childWidth;
    }}
  }}
  layoutBranch(frame,0,1);
  showDetails(frame);
  zoomHint.textContent='Expanded callees under '+frame.getAttribute('title')+' · self time excluded';
}}
function reset(){{for(const item of frames){{
  item.style.display='';
  place(item,Number(item.dataset.x),Number(item.dataset.width));
}}}}
frames.forEach(item=>{{
  item.addEventListener('click',()=>zoom(item));
  item.addEventListener('mouseenter',()=>showDetails(item));
  item.addEventListener('focus',()=>showDetails(item));
}});
reset();
addEventListener('resize',()=>{{
  for(const item of frames){{
    if(item.style.display==='none')continue;
    const width=item.getBoundingClientRect().width;
    const label=item.querySelector('.frame-label');
    if(label)label.style.visibility=(width>=140)?'visible':'hidden';
  }}
}});
document.getElementById('reset-zoom').addEventListener('click',reset);
document.getElementById('reset-zoom').addEventListener('click',()=>{{
  zoomHint.textContent='Showing the full profile';
  details.textContent='Hover over a frame for its function and sample count. Click to zoom.';
}});
document.querySelectorAll('#frame-index button').forEach(button=>button.addEventListener('click',()=>{{
  const frame=document.getElementById(button.dataset.target);zoom(frame);frame.scrollIntoView({{block:'center'}});
}}));
document.querySelectorAll('.narrow-calls button').forEach(button=>button.addEventListener('click',()=>{{
  const frame=document.getElementById(button.dataset.target);zoom(frame);frame.scrollIntoView({{block:'center'}});
}}));
document.getElementById('frame-filter').addEventListener('input',event=>{{
  const query=event.target.value.toLowerCase();
  let shown=0;
  document.querySelectorAll('#frame-index li').forEach(item=>{{
    item.hidden=!item.textContent.toLowerCase().includes(query);
    if(!item.hidden)shown++;
  }});
  document.getElementById('empty-state').hidden=shown!==0;
}});
</script></body></html>'''
    output.write_text(document)


def main():
    if len(sys.argv) != 4:
        fail("usage: render-flamegraph.py PROFILE_TRACE KERNEL_EFI OUTPUT_HTML")
    trace, binary, output = map(Path, sys.argv[1:])
    if not binary.is_file():
        fail(f"kernel symbol file not found: {binary}")
    lines = trace.read_text(errors="replace").splitlines()
    if "AGNOSTOS_PROFILE_V1" not in lines or "AGNOSTOS_PROFILE_END" not in lines:
        fail("profile trace is incomplete or has an unsupported format")
    samples = []
    addresses = set()
    anchor_line = next((line for line in lines if line.startswith("AGNOSTOS_PROFILE_ANCHOR=")), None)
    if not anchor_line:
        fail("profile trace has no symbolization anchor")
    anchor = int(anchor_line.split("=", 1)[1], 16)
    for line in lines:
        if not line.startswith("P:"):
            continue
        try:
            stack = [int(item, 16) for item in line[2:].split()]
        except ValueError:
            continue
        if stack:
            samples.append(stack)
            addresses.update(stack)
    if not samples:
        fail("profile trace contains no samples")
    names = symbolize(binary, addresses, anchor)
    root = {"samples": 0, "children": {}}
    for stack in samples:
        # The first IP is the interrupted leaf; subsequent entries walk toward
        # callers, so reverse to build the conventional root-to-leaf graph.
        resolved = [names[address] or "[unknown]" for address in reversed(stack)]
        resolved = [
            name for name in resolved
            if name == "[unknown]" or (
                "profiling::" not in name
                and "interrupts::handlers::timer" not in name
            )
        ]
        if any(name != "[unknown]" for name in resolved):
            add_stack(root, resolved)
    if not root["samples"]:
        fail("none of the captured stacks resolved to kernel functions")
    output.parent.mkdir(parents=True, exist_ok=True)
    render(root, output)
    print(f"Rendered {root['samples']} resolved samples (of {len(samples)} captured) to {output}")


if __name__ == "__main__":
    main()
