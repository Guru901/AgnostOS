#!/usr/bin/env python3
"""Render a zoomable flame graph from AgnostOS aggregate call-tree data."""

import html
import json
import sys
from pathlib import Path


def fail(message):
    raise SystemExit(message)


def render(trace, output):
    lines = trace.read_text(errors="replace").splitlines()
    if "AGNOSTOS_PROFILE_V2" not in lines or "AGNOSTOS_PROFILE_END" not in lines:
        fail("profile trace is incomplete or has an unsupported format")

    nodes = {
        0: {
            "id": 0,
            "parent": None,
            "name": "[root]",
            "calls": 0,
            "weight": 0,
            "children": [],
        }
    }
    dropped = 0
    for line in lines:
        if line.startswith("N:"):
            try:
                node_id_text, parent_text, calls_text, name = line[2:].split(" ", 3)
                node_id = int(node_id_text)
                parent = int(parent_text)
                calls = int(calls_text)
            except (ValueError, TypeError):
                fail(f"invalid call-tree record: {line!r}")
            nodes[node_id] = {
                "id": node_id,
                "parent": parent,
                "name": name,
                "calls": calls,
                "weight": calls,
                "children": [],
            }
        elif line.startswith("AGNOSTOS_PROFILE_DROPPED_PATHS="):
            try:
                dropped = int(line.split("=", 1)[1])
            except ValueError:
                fail("profile trace has an invalid dropped-path count")

    for node in list(nodes.values())[1:]:
        parent = nodes.get(node["parent"])
        if parent is None:
            fail(f"call-tree node {node['id']} refers to missing parent {node['parent']}")
        parent["children"].append(node["id"])

    def calculate_weight(node_id):
        node = nodes[node_id]
        node["weight"] = node["calls"] + sum(
            calculate_weight(child_id) for child_id in node["children"]
        )
        return node["weight"]

    total = calculate_weight(0)
    max_depth = 0

    def measure_depth(node_id, depth=0):
        nonlocal max_depth
        for child_id in nodes[node_id]["children"]:
            max_depth = max(max_depth, depth + 1)
            measure_depth(child_id, depth + 1)

    measure_depth(0)
    row_height = 26
    frame_height = 24
    chart_height = max(max_depth, 1) * row_height
    colors = ("#f2c14e", "#f78154", "#e85d75", "#c44569", "#f4a261")
    frames = []
    def emit_frame(node_id, x, y, width, depth):
        node = nodes[node_id]
        name = node["name"]
        call_word = "call" if node["calls"] == 1 else "calls"
        subtree_word = "call" if node["weight"] == 1 else "calls"
        title = html.escape(
            f"{name} — {node['calls']:,} {call_word}, "
            f"{node['weight']:,} {subtree_word} in subtree"
        )
        frames.append(
            f'<button type="button" class="frame" id="frame-{node_id}" '
            f'data-node="{node_id}" title="{title}" '
            f'style="left:{x * 100:.6f}%;width:{width * 100:.6f}%;top:{y}px;'
            f'height:{frame_height}px;background:{colors[depth % len(colors)]}">'
            f'<span>{html.escape(name)}</span></button>'
        )
        cursor = x
        child_weight = sum(nodes[child_id]["weight"] for child_id in node["children"])
        for child_id in sorted(
            node["children"], key=lambda child: (-nodes[child]["weight"], nodes[child]["name"])
        ):
            child = nodes[child_id]
            child_width = width * child["weight"] / max(child_weight, 1)
            emit_frame(child_id, cursor, y - row_height, child_width, depth + 1)
            cursor += child_width

    root_children = sorted(
        nodes[0]["children"], key=lambda child: (-nodes[child]["weight"], nodes[child]["name"])
    )
    cursor = 0.0
    root_weight = sum(nodes[node_id]["weight"] for node_id in root_children)
    for node_id in root_children:
        node = nodes[node_id]
        width = node["weight"] / max(root_weight, 1)
        emit_frame(node_id, cursor, chart_height - row_height, width, 0)
        cursor += width

    function_paths = []
    for node_id, node in nodes.items():
        if node_id == 0:
            continue
        names = []
        current = node
        while current["parent"] is not None:
            names.append(current["name"])
            current = nodes[current["parent"]]
        names.reverse()
        function_paths.append((node_id, " → ".join(names), node["calls"]))
    index = "".join(
        f'<li><button type="button" data-target="{node_id}">'
        f'<span class="name">{html.escape(path)}</span>'
        f'<span class="count">{count:,} {"call" if count == 1 else "calls"}</span></button></li>'
        for node_id, path, count in sorted(function_paths, key=lambda item: (-item[2], item[1]))
    ) or '<li class="empty">No profiled calls were recorded.</li>'
    tree_json = json.dumps(nodes, separators=(",", ":")).replace("</", "<\\/")
    warning = (
        f'<p class="warning">The call-tree node limit was reached {dropped:,} times; '
        "some paths were omitted.</p>"
        if dropped
        else ""
    )

    document = f'''<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>AgnostOS Flame Graph</title>
<style>
:root{{--ink:#20242c;--muted:#667085;--line:#e4e7ec;--paper:#fff;--wash:#f4f6f8}}
*{{box-sizing:border-box}}body{{margin:0;background:var(--wash);color:var(--ink);font:14px/1.45 system-ui,-apple-system,"Segoe UI",sans-serif}}
main{{max-width:1500px;margin:0 auto;padding:28px clamp(16px,3vw,40px) 48px}}
header{{display:flex;align-items:center;justify-content:space-between;gap:20px;margin-bottom:16px}}
h1{{font-size:22px;line-height:1.2;margin:0 0 5px}}p{{margin:0;color:var(--muted)}}.total{{white-space:nowrap;border:1px solid var(--line);background:var(--paper);border-radius:999px;padding:8px 12px;font-weight:650}}
.card{{background:var(--paper);border:1px solid var(--line);border-radius:12px;box-shadow:0 1px 2px #1018280a;padding:18px}}
.toolbar{{display:flex;justify-content:space-between;align-items:center;gap:12px;margin-bottom:12px;color:var(--muted)}}
.chart{{position:relative;width:100%;height:{chart_height}px;overflow:hidden;border:1px solid #edf0f2;border-radius:8px;background:#fff}}
.frame{{position:absolute;overflow:hidden;white-space:nowrap;border:1px solid #fff;border-radius:2px;padding:1px 6px;color:var(--ink);font:11px/20px ui-monospace,SFMono-Regular,Consolas,monospace;text-align:left;cursor:pointer}}
.frame span{{display:block;overflow:hidden;text-overflow:ellipsis;pointer-events:none}}.frame:hover,.frame:focus{{outline:1.5px solid #20242c;outline-offset:-1.5px;z-index:2}}
.toolbar button,.functions-head button{{border:1px solid #cfd4dc;background:#fff;color:var(--ink);border-radius:7px;padding:6px 10px;font:inherit;font-weight:600;cursor:pointer}}
.functions{{margin-top:16px;overflow:hidden}}.functions-head{{display:flex;justify-content:space-between;align-items:center;gap:14px;margin-bottom:8px}}
h2{{font-size:16px;margin:0}}ul{{list-style:none;margin:0;padding:0;max-height:320px;overflow:auto}}li+li{{border-top:1px solid #f0f1f3}}
.functions li button{{display:flex;width:100%;justify-content:space-between;gap:16px;text-align:left;border:0;background:#fff;padding:9px 4px;color:var(--ink);cursor:pointer}}
.functions li button:hover,.functions li button:focus{{background:#fff8eb;outline:none}}.name{{font:12px/1.5 ui-monospace,SFMono-Regular,Consolas,monospace;overflow-wrap:anywhere}}
.count{{flex:none;color:var(--muted);font-size:12px}}.empty{{padding:10px 4px;color:var(--muted)}}.warning{{margin-top:14px;color:#b54708}}
@media(max-width:640px){{main{{padding:16px 10px}}header{{align-items:flex-start;flex-direction:column}}.card{{padding:12px}}.functions-head{{align-items:flex-start;flex-direction:column}}}}
</style></head><body><main>
<header><div><h1>AgnostOS flame graph</h1><p>Aggregated call paths from profiled functions · widths show call counts, not time</p></div>
<div class="total">{total:,} calls</div></header>
<section class="card" aria-label="Flame graph"><div class="toolbar"><span id="zoom-hint">Showing the full call tree</span><button id="reset" type="button">Reset zoom</button></div>
<div class="chart" id="chart">{''.join(frames)}</div>{warning}</section>
<section class="card functions"><div class="functions-head"><h2>Call paths</h2><button id="show-all" type="button">Show all</button></div>
<ul>{index}</ul></section>
</main><script>
const nodes={tree_json};
const frames=[...document.querySelectorAll('.frame')];
const chart=document.getElementById('chart');
const hint=document.getElementById('zoom-hint');
const rowHeight={row_height};
const colors={json.dumps(colors)};
function isUnder(candidate,ancestor){{
  let current=nodes[candidate];
  while(current&&current.parent!==null){{
    if(current.id===ancestor)return true;
    current=nodes[current.parent];
  }}
  return false;
}}
function place(node,x,y,width,depth){{
  const frame=document.getElementById('frame-'+node.id);
  frame.style.display='';frame.style.left=(x*100)+'%';frame.style.width=(width*100)+'%';
  frame.style.top=y+'px';frame.style.background=colors[depth%colors.length];
  let cursor=x;
  const children=node.children.map(id=>nodes[id]).sort((a,b)=>b.weight-a.weight||a.name.localeCompare(b.name));
  const childWeight=children.reduce((sum,child)=>sum+child.weight,0);
  for(const child of children){{
    const childWidth=width*child.weight/Math.max(childWeight,1);
    place(child,cursor,y-rowHeight,childWidth,depth+1);cursor+=childWidth;
  }}
}}
function zoom(id){{
  const selected=nodes[id];if(!selected)return;
  for(const frame of frames)frame.style.display=isUnder(Number(frame.dataset.node),id)?'':'none';
  place(selected,0,chart.clientHeight-rowHeight,1,0);
  hint.textContent='Expanded calls under '+selected.name+' · click a frame to zoom further';
}}
function reset(){{
  for(const frame of frames)frame.style.display='';
  let cursor=0;
  const children=nodes[0].children.map(id=>nodes[id]).sort((a,b)=>b.weight-a.weight||a.name.localeCompare(b.name));
  const rootWeight=children.reduce((sum,child)=>sum+child.weight,0);
  for(const child of children){{
    const width=child.weight/Math.max(rootWeight,1);
    place(child,cursor,chart.clientHeight-rowHeight,width,0);cursor+=width;
  }}
  hint.textContent='Showing the full call tree';
}}
frames.forEach(frame=>frame.addEventListener('click',()=>zoom(Number(frame.dataset.node))));
document.querySelectorAll('.functions li button').forEach(button=>button.addEventListener('click',()=>zoom(Number(button.dataset.target))));
document.getElementById('reset').addEventListener('click',reset);
document.getElementById('show-all').addEventListener('click',()=>{{
  reset();
}});
reset();
</script></body></html>'''
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(document)
    print(f"Rendered {total:,} aggregate calls across {len(nodes) - 1:,} call paths to {output}")


def main():
    if len(sys.argv) != 3:
        fail("usage: render-flamegraph.py PROFILE_TRACE OUTPUT_HTML")
    render(Path(sys.argv[1]), Path(sys.argv[2]))


if __name__ == "__main__":
    main()
