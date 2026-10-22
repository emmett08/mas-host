#!/usr/bin/env python3
"""Render the README architecture as GIF, PNG and SVG from one scene.

Requires Pillow 12.3.0 and DejaVu Sans. No host, provider or detector is run.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
from html import escape
import math
from pathlib import Path
import textwrap

from PIL import Image, ImageDraw, ImageFont


WIDTH, HEIGHT, SCALE = 1200, 1130, 2
FPS, STAGE_SECONDS = 10, 4
BG, PANEL, CARD = "#0c1424", "#111e32", "#17263d"
INK, MUTED, LINE = "#eff5ff", "#b8c8df", "#47617e"
BLUE, TEAL, AMBER, PURPLE = "#78b7ff", "#73dfc4", "#ffc979", "#c2a8ff"


@dataclass(frozen=True)
class Node:
    x: int
    y: int
    w: int
    h: int
    title: str
    lines: tuple[str, ...]
    colour: str = BLUE


NODES = {
    "app": Node(58, 133, 340, 72, "Application project", ("Source + agents + topology",)),
    "build": Node(444, 133, 242, 72, "Build tooling", ("Validate + bind handlers",)),
    "bundle": Node(732, 133, 410, 72, "Frozen bundle + executables", ("Schemas, routes and executable digests",)),
    "client": Node(58, 290, 240, 102, "CLI / optional A2A", ("A2A: authenticated tasks", "Scoped Agent Cards")),
    "core": Node(368, 290, 440, 112, "Rust execution core", ("Identity, routes and run budgets", "Named requests, replay and effects")),
    "policy": Node(872, 290, 270, 112, "Governed boundaries", ("Current grants + policy", "Baseline + source policies"), AMBER),
    "observe": Node(58, 462, 240, 76, "Run observation", ("mas-host observe",), PURPLE),
    "protocol": Node(368, 472, 210, 72, "protocol.sqlite", ("Messages + tasks",), TEAL),
    "journal": Node(598, 472, 210, 72, "execution.sqlite", ("Steps + sources",), TEAL),
    "inspect": Node(872, 462, 270, 82, "Privacy inspector", ("Pinned model + patterns",), AMBER),
    "sources": Node(872, 566, 270, 96, "Source restrictions", ("Ancestry + task exposure", "Expiry + pending acquisition"), AMBER),
    "reader": Node(58, 578, 240, 96, "Authorised readers", ("GetRetainedEffect", "GetRetainedSource"), PURPLE),
    "native": Node(78, 768, 164, 76, "Native SDK", ("Go / Rust", "C++ / Python"), TEAL),
    "command": Node(260, 768, 164, 76, "Command", ("CLI / MCP",), TEAL),
    "llm": Node(442, 768, 164, 76, "LLM", ("Providers", "Scoped secrets"), TEAL),
    "workflow": Node(624, 768, 164, 76, "Workflow", ("Fork / join", "Branch / loop"), TEAL),
    "external": Node(872, 746, 270, 112, "External systems", ("Model providers / tools", "Worker executes the effect"), AMBER),
}


@dataclass(frozen=True)
class Edge:
    points: tuple[tuple[int, int], ...]
    colour: str = BLUE
    dashed: bool = False


EDGES = {
    "build": Edge(((398, 169), (444, 169))),
    "bundle": Edge(((686, 169), (732, 169))),
    "load": Edge(((936, 205), (936, 231), (770, 231), (770, 290))),
    "submit": Edge(((298, 343), (368, 343))),
    "policy": Edge(((808, 343), (872, 343)), AMBER, True),
    "inspection": Edge(((1007, 402), (1007, 462)), AMBER, True),
    "protocol": Edge(((473, 402), (473, 472)), TEAL),
    "journal": Edge(((703, 402), (703, 472)), TEAL),
    "observe_protocol": Edge(((368, 508), (298, 508)), PURPLE, True),
    "observe_journal": Edge(((598, 523), (588, 523), (588, 558), (178, 558), (178, 538)), PURPLE, True),
    "retained": Edge(((703, 544), (703, 616), (298, 616)), PURPLE, True),
    "read_request": Edge(((58, 362), (46, 362), (46, 616), (58, 616)), PURPLE, True),
    "source_record": Edge(((808, 523), (854, 523), (854, 614), (872, 614)), AMBER, True),
    "source_policy": Edge(((1142, 614), (1152, 614), (1152, 370), (1142, 370)), AMBER, True),
    "external": Edge(((524, 844), (524, 884), (1007, 884), (1007, 858)), AMBER),
    "acquire": Edge(((160, 844), (160, 884), (1007, 884), (1007, 858)), AMBER),
}
for worker, x in (("native", 160), ("command", 342), ("llm", 524), ("workflow", 706)):
    EDGES[worker] = Edge(((808, 381), (840, 381), (840, 750), (x, 750), (x, 768)), TEAL)


@dataclass(frozen=True)
class Flow:
    edge: str
    start: float = 0
    end: float = 1
    reverse: bool = False


@dataclass(frozen=True)
class Stage:
    title: str
    detail: str
    nodes: tuple[str, ...]
    flows: tuple[Flow, ...]


STAGES = (
    Stage("Build and freeze an application", "Application bindings and topology become validated launchers and a frozen bundle. The host checks executable and input identity on execution and replay.", ("app", "build", "bundle"), (Flow("build", 0, .45), Flow("bundle", .5, 1))),
    Stage("Admit a run through the shared core", "The local CLI and optional A2A service use the same execution core. A2A adds authenticated, principal-scoped task records in its separate service journal.", ("bundle", "client", "core"), (Flow("load", 0, .45), Flow("submit", .5, 1))),
    Stage("Keep protocol and execution responsibilities separate", "Nehemiah owns message admission, tasks and call reservations. The host journals steps, effects, source associations and invocation exposure in execution.sqlite. Each database retains its own transaction boundary.", ("core", "protocol", "journal"), (Flow("protocol", 0, .45), Flow("journal", .5, 1))),
    Stage("Start a supervised worker", "The guardian starts a child process with private mas-native/1 IPC. Native, command, LLM and workflow are selectable worker kinds under the same host contract.", ("core", "workflow"), (Flow("workflow"),)),
    Stage("Delegate through the host", "A workflow requests a child over IPC. The host admits the logical Nehemiah request, then dispatches the selected worker. Branch timing and packet motion are illustrative.", ("workflow", "core", "protocol", "llm"), (Flow("workflow", 0, .35, True), Flow("protocol", .37, .55), Flow("llm", .58, 1))),
    Stage("Record effect admission before external execution", "The worker requests permission. Durable host admission precedes the external action. Completion returns with evidence. A lost completion response retains uncertainty for reconciliation.", ("llm", "core", "journal", "external"), (Flow("llm", 0, .15, True), Flow("journal", .16, .28), Flow("llm", .29, .43), Flow("external", .44, .60), Flow("external", .61, .74, True), Flow("llm", .75, .9, True), Flow("journal", .91, 1))),
    Stage("Apply configured governance at protected boundaries", "The governed A2A profile checks current policy and structured projections. When configured, the pinned local inspector gates admission, effects, retention and delivery. Failed checks stop release.", ("core", "policy", "inspect"), (Flow("policy", 0, .2), Flow("inspection", .25, .5), Flow("inspection", .55, .8, True), Flow("policy", .85, 1, True))),
    Stage("Acquire a source under a pending obligation", "PR #20: pending acquisition is retained before effect permission and blocks new outbound work. Completion seals the retained projection, source association and recipient exposure in the existing journal transaction.", ("native", "core", "journal", "sources", "external"), (Flow("journal", 0, .13), Flow("native", .14, .3), Flow("acquire", .31, .5), Flow("native", .51, .73, True), Flow("journal", .74, .85), Flow("source_record", .86, 1))),
    Stage("Carry source restrictions into subsequent work", "In v2 namespaces, complete source ancestry and task exposure constrain child requests, effects, detector processing and delivery. Original policy and expiry follow the data. Missing lineage, expiry or a closure above 64 sources blocks admission.", ("journal", "sources", "policy", "core", "workflow", "llm"), (Flow("source_policy", 0, .2), Flow("policy", .21, .35, True), Flow("workflow", .36, .55), Flow("workflow", .56, .7, True), Flow("llm", .71, 1))),
    Stage("Read retained effects and source projections", "Both readers derive scope from an authorised A2A task. V2 adds complete source/task exposure to current resolve authority and configured inspection. GetRetainedSource returns the retained projection with original bytes marked not-retained.", ("client", "reader", "journal", "sources", "policy", "inspect"), (Flow("read_request", 0, .15), Flow("source_policy", .16, .3), Flow("policy", .31, .4), Flow("inspection", .41, .55), Flow("retained", .56, .8), Flow("read_request", .82, 1, True))),
    Stage("Reconstruct privacy case status conservatively", "The privacy adapter combines two independent retained-effect views. Host settlement, payload disposition and external execution certainty stay distinct. Recipient delivery remains unknown.", ("reader", "journal", "client"), (Flow("retained", 0, .3), Flow("retained", .35, .65), Flow("read_request", .7, 1, True))),
    Stage("Observe ordinary runs without executing", "mas-host observe reads each database separately. Payloads require explicit opt-in. Governed namespaces use their authorised read boundary. Observing performs no recovery or external action.", ("observe", "protocol", "journal"), (Flow("observe_protocol", 0, .45), Flow("observe_journal", .5, 1))),
)


class Canvas:
    """One set of drawing operations produces both raster and vector output."""

    def __init__(self, font_dir: Path):
        self.image = Image.new("RGB", (WIDTH * SCALE, HEIGHT * SCALE), BG)
        self.draw = ImageDraw.Draw(self.image)
        self.fonts = {}
        self.font_dir = font_dir
        self.svg = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{HEIGHT}" viewBox="0 0 {WIDTH} {HEIGHT}" role="img" aria-labelledby="title desc">', '<title id="title">MAS Host for Nehemiah: architecture including PR #20</title>', '<desc id="desc">Application build inputs produce a frozen bundle. CLI or A2A ingress reaches the Rust core. Separate protocol and execution stores support supervised workers, replay and admitted effects. Source associations and invocation exposure stay in execution.sqlite. Optional governance carries complete source restrictions and expiry through later work. Authorised effect and source readers apply current authority and task exposure. Privacy case status preserves uncertainty. Ordinary observation is read-only.</desc>']
        self.rect(0, 0, WIDTH, HEIGHT, BG, radius=0)

    def font(self, size, bold=False):
        key = (size, bold)
        if key not in self.fonts:
            suffix = "-Bold" if bold else ""
            self.fonts[key] = ImageFont.truetype(str(self.font_dir / f"DejaVuSans{suffix}.ttf"), size * SCALE)
        return self.fonts[key]

    def text(self, x, y, value, size=18, colour=INK, bold=False):
        self.draw.text((x * SCALE, y * SCALE), value, font=self.font(size, bold), fill=colour, anchor="lt")
        self.svg.append(f'<text x="{x}" y="{y}" dominant-baseline="text-before-edge" font-family="DejaVu Sans, sans-serif" font-size="{size}" font-weight="{700 if bold else 400}" fill="{colour}">{escape(value)}</text>')

    def rect(self, x, y, w, h, fill, outline=None, radius=12, width=1):
        self.draw.rounded_rectangle((x * SCALE, y * SCALE, (x + w) * SCALE, (y + h) * SCALE), radius * SCALE, fill, outline, width * SCALE)
        self.svg.append(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{radius}" fill="{fill}" stroke="{outline or "none"}" stroke-width="{width}"/>')

    def line(self, points, colour=LINE, width=2, dashed=False):
        for a, b in zip(points, points[1:]):
            length = math.dist(a, b)
            if dashed:
                for start in range(0, math.ceil(length), 13):
                    end = min(start + 7, length)
                    pair = tuple(((a[0] + (b[0] - a[0]) * t / length) * SCALE, (a[1] + (b[1] - a[1]) * t / length) * SCALE) for t in (start, end))
                    self.draw.line(pair, colour, width * SCALE)
            else:
                self.draw.line(tuple((x * SCALE, y * SCALE) for x, y in (a, b)), colour, width * SCALE)
        coords = " ".join(f"{x},{y}" for x, y in points)
        dash = ' stroke-dasharray="7 6"' if dashed else ""
        self.svg.append(f'<polyline points="{coords}" fill="none" stroke="{colour}" stroke-width="{width}" stroke-linejoin="round"{dash}/>')

    def polygon(self, points, fill, outline=None):
        self.draw.polygon(tuple((x * SCALE, y * SCALE) for x, y in points), fill)
        if outline:
            self.draw.line(tuple((x * SCALE, y * SCALE) for x, y in (*points, points[0])), outline, SCALE)
        coords = " ".join(f"{x},{y}" for x, y in points)
        self.svg.append(f'<polygon points="{coords}" fill="{fill}" stroke="{outline or "none"}"/>')

    def dot(self, x, y, colour):
        self.draw.ellipse(((x - 7) * SCALE, (y - 7) * SCALE, (x + 7) * SCALE, (y + 7) * SCALE), colour, BG, 2 * SCALE)

    def arrow(self, edge, colour):
        a, b = edge.points[-2:]
        angle = math.atan2(b[1] - a[1], b[0] - a[0])
        self.polygon((b, (b[0] - 9 * math.cos(angle - .45), b[1] - 9 * math.sin(angle - .45)), (b[0] - 9 * math.cos(angle + .45), b[1] - 9 * math.sin(angle + .45))), colour)

    def finish(self):
        return self.image.resize((WIDTH, HEIGHT), Image.Resampling.LANCZOS)


def point_on(edge, progress):
    lengths = [math.dist(a, b) for a, b in zip(edge.points, edge.points[1:])]
    remaining = sum(lengths) * progress
    for a, b, length in zip(edge.points, edge.points[1:], lengths):
        if remaining <= length:
            t = remaining / length
            return a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t
        remaining -= length
    return edge.points[-1]


def render(font_dir, stage=None, progress=0):
    c = Canvas(font_dir)
    c.rect(38, 33, 5, 54, TEAL, radius=2)
    c.text(58, 32, "MAS Host for Nehemiah", 32, bold=True)
    c.text(58, 78, "Build once. Admit explicitly. Retain enough evidence to recover.", 19, MUTED)
    c.text(58, 113, "APPLICATION BUILD", 12, TEAL, True)
    c.polygon(((36, 252), (1164, 252), (1164, 676), (848, 676), (848, 920), (36, 920)), PANEL, LINE)
    c.text(58, 264, "TRUSTED LOCAL LINUX HOST", 12, MUTED, True)
    c.text(872, 264, "OPTIONAL GOVERNED A2A PROFILE", 12, AMBER, True)
    c.rect(350, 438, 476, 124, PANEL, LINE)
    c.text(368, 449, "TWO STORES", 12, TEAL, True)
    c.rect(58, 698, 750, 166, PANEL, LINE)
    c.text(78, 715, "SUPERVISED WORKER PROCESSES", 15, TEAL, True)
    c.text(368, 680, "mas-native/1 · Unix socketpair · framed JSON", 17, MUTED)
    c.text(872, 716, "OUTSIDE THE HOST", 12, AMBER, True)
    c.text(78, 897, "Original deadlines · process-group cleanup · trusted code", 17, MUTED)

    active_edges = {flow.edge for flow in stage.flows} if stage else set()
    for name, edge in sorted(EDGES.items(), key=lambda item: item[0] in active_edges):
        colour = edge.colour if name in active_edges else LINE
        c.line(edge.points, colour, 3 if name in active_edges else 2, edge.dashed)
        current = next((f for f in stage.flows if f.edge == name and f.start <= progress <= f.end), None) if stage else None
        arrow_edge = Edge(tuple(reversed(edge.points))) if current and current.reverse else edge
        c.arrow(arrow_edge, colour)

    for name, n in NODES.items():
        active = stage is not None and name in stage.nodes
        c.rect(n.x, n.y, n.w, n.h, CARD, n.colour if active else LINE, width=3 if active else 1)
        c.rect(n.x + 12, n.y + 14, 3, n.h - 28, n.colour if active else LINE, radius=1)
        size = 18 if n.w < 280 else 21
        c.text(n.x + 25, n.y + 13, n.title, size, INK, True)
        for index, line in enumerate(n.lines):
            c.text(n.x + 25, n.y + 38 + index * 20, line, 16, MUTED)

    if stage:
        for flow in stage.flows:
            if flow.start <= progress <= flow.end:
                t = (progress - flow.start) / (flow.end - flow.start)
                t = t * t * (3 - 2 * t)
                if flow.reverse:
                    t = 1 - t
                edge = EDGES[flow.edge]
                c.dot(*point_on(edge, t), edge.colour)

    c.rect(36, 940, 1128, 132, PANEL, LINE)
    if stage:
        index = STAGES.index(stage)
        title = f"{index + 1:02d} / {len(STAGES)}   {stage.title}"
        detail = stage.detail
    else:
        title = "Architecture at a glance"
        detail = "The host mediates communication and durable execution. Governed acquisition carries source policy, ancestry and exposure through later work. Authorised reads preserve settlement, payload disposition and external execution certainty as separate facts."
    c.text(58, 958, title, 21, INK, True)
    for index, line in enumerate(textwrap.wrap(detail, width=110)):
        c.text(58, 996 + index * 23, line, 18, MUTED)
    if stage:
        x = 58 + (1106 / len(STAGES)) * STAGES.index(stage)
        c.rect(x, 1064, 1106 / len(STAGES) * progress, 3, TEAL, radius=0)
    for x, colour, label, dashed in ((58, BLUE, "Build / ingress", False), (302, TEAL, "Execution / IPC", False), (568, AMBER, "Governance / effects", True), (898, PURPLE, "Read-only paths", True)):
        c.line(((x, 1101), (x + 25, 1101)), colour, 3, dashed)
        c.text(x + 35, 1090, label, 16, MUTED)
    c.svg.append("</svg>")
    return c


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, default=Path(__file__).parent)
    parser.add_argument("--font-dir", type=Path, default=Path("/usr/share/fonts/truetype/dejavu"))
    parser.add_argument("--contact-sheet", type=Path, help="Optional review image outside the committed assets")
    args = parser.parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    overview = render(args.font_dir)
    overview.finish().save(args.output_dir / "mas-host-architecture.png")
    (args.output_dir / "mas-host-architecture.svg").write_text("\n".join(overview.svg) + "\n", encoding="utf-8")

    # Fixed shared palette prevents colour flicker and keeps frame deltas small.
    samples = [overview.finish()]
    samples.extend(render(args.font_dir, s, .5).finish() for s in STAGES)
    strip = Image.new("RGB", (WIDTH, HEIGHT * len(samples)))
    for i, sample in enumerate(samples):
        strip.paste(sample, (0, i * HEIGHT))
    # Reserve semantic colours explicitly: area-weighted quantisation alone
    # can discard a thin amber/purple line among the much larger navy panels.
    exact_colours = (BG, PANEL, CARD, INK, MUTED, LINE, BLUE, TEAL, AMBER, PURPLE)
    median = strip.quantize(colors=128 - len(exact_colours), method=Image.Quantize.MEDIANCUT)
    exact = [value for colour in exact_colours for value in bytes.fromhex(colour[1:])]
    palette = Image.new("P", (1, 1))
    palette.putpalette(exact + median.getpalette()[:3 * (128 - len(exact_colours))])
    del strip
    quantize = lambda frame: frame.quantize(palette=palette, dither=Image.Dither.NONE)
    frames, durations = [quantize(samples[0])], [2500]
    for stage in STAGES:
        for i in range(FPS * STAGE_SECONDS):
            frames.append(quantize(render(args.font_dir, stage, i / (FPS * STAGE_SECONDS - 1)).finish()))
            durations.append(1000 // FPS)
        durations[-1] += 2000  # Hold the final frame so the caption can be read.
        print(f"Rendered {STAGES.index(stage) + 1}/{len(STAGES)}: {stage.title}", flush=True)
    frames.append(frames[0])
    durations.append(2000)
    frames[0].save(args.output_dir / "mas-host-architecture.gif", save_all=True, append_images=frames[1:], duration=durations, loop=0, optimize=True, disposal=1)
    if args.contact_sheet:
        sheet = Image.new("RGB", (WIDTH, 3 * HEIGHT // 2), BG)
        for i, image in enumerate(samples[:6]):
            sheet.paste(image.resize((WIDTH // 2, HEIGHT // 2)), ((i % 2) * WIDTH // 2, (i // 2) * HEIGHT // 2))
        sheet.save(args.contact_sheet)
    for path in sorted(args.output_dir.glob("mas-host-architecture.*")):
        print(f"{path.name}: {path.stat().st_size:,} bytes")


if __name__ == "__main__":
    main()
