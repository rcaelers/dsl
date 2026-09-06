#!/usr/bin/env python3
"""Summarize process-owned Metal intervals exported by xctrace (Python stdlib only).

Command-buffer spans are not UI frames or presentation latency. Concurrent GPU
channels are merged into a union, never summed as elapsed time. XML references
are document-local. Unknown schemas/references or malformed target rows fail.
"""
import argparse
import collections
import json
import math
import xml.etree.ElementTree as ET


class Table:
    def __init__(self, path, schema):
        self.root = ET.parse(path).getroot()
        tables = self.root.findall("node")
        if (len(tables) != 1 or tables[0].find("schema") is None
                or tables[0].find("schema").get("name") != schema):
            raise ValueError("Expected exactly one " + schema + " table")
        self.table = tables[0]
        self.ids = {}
        for element in self.root.iter():
            if "id" in element.attrib:
                key = element.get("id")
                if key in self.ids:
                    raise ValueError("Duplicate XML identity")
                self.ids[key] = element
        self.columns = [c.text for c in self.table.findall("schema/col/mnemonic")]
        if len(set(self.columns)) != len(self.columns):
            raise ValueError("Duplicate columns")

    def resolve(self, element):
        if element is None:
            raise ValueError("Missing XML value")
        seen = set()
        while "ref" in element.attrib:
            key = element.get("ref")
            if key in seen or key not in self.ids:
                raise ValueError("Invalid XML reference")
            seen.add(key)
            element = self.ids[key]
        return element

    def rows(self, pid):
        if "process" not in self.columns:
            raise ValueError("Table cannot be process-filtered")
        for row in self.table.findall("row"):
            if len(row) != len(self.columns):
                raise ValueError("Incomplete row")
            values = dict(zip(self.columns, map(self.resolve, row)))
            process = values["process"]
            if process.tag == "sentinel":
                continue
            if number(self.resolve(process.find("pid"))) == pid:
                yield values


def number(element):
    if element.tag == "sentinel" or element.text is None:
        raise ValueError("Missing numeric value")
    value = int(element.text)
    if value < 0:
        raise ValueError("Negative timestamp or identity")
    return value


def union_ns(intervals):
    total = 0
    previous_end = 0
    for start, end in sorted(intervals):
        if start < 0 or end < start:
            raise ValueError("Invalid interval")
        total += max(0, end - max(start, previous_end))
        previous_end = max(previous_end, end)
    return total


def distribution(values):
    if not values or any(not math.isfinite(v) or v < 0 for v in values):
        raise ValueError("Empty or invalid distribution")
    values = sorted(values)
    return {"count": len(values), "samples_ms": values,
            **{f"p{p}_ms": values[math.ceil(len(values) * p / 100) - 1] for p in (50, 95, 99)},
            "max_ms": values[-1]}


def summarize(gpu, application, pid, start_ns, end_ns, required_labels=()):
    if pid <= 0 or start_ns < 0 or end_ns <= start_ns:
        raise ValueError("Invalid process or analysis window")
    # Obtain labels from typed metadata, not formatted strings containing process names.
    labels = collections.defaultdict(set)
    for row in application.rows(pid):
        if row["event-type"].text != "Encoding":
            continue
        label = row["event-label"].find("metal-object-label")
        # The command-buffer label includes a frame identity; encoder labels do not.
        if label is not None and row["event-label"].find("uint64") is not None:
            labels[number(row["cmdbuffer-id"])].add(application.resolve(label).text or "")

    groups = collections.defaultdict(list)
    excluded = collections.Counter()
    for row in gpu.rows(pid):
        if row["state"].text != "Active" or number(row["event-depth"]) != 0:
            excluded["non_top_level_active_rows"] += 1
            continue
        start = number(row["start"])
        duration = number(row["duration"])
        if not row["channel-name"].text:
            raise ValueError("Missing GPU channel")
        groups[number(row["cmdbuffer-id"])].append({
            "start_ns": start, "duration_ns": duration,
            "channel": row["channel-name"].text,
        })

    buffers = []
    for identity, rows in groups.items():
        intervals = [(r["start_ns"], r["start_ns"] + r["duration_ns"]) for r in rows]
        first = min(s for s, _ in intervals)
        last = max(e for _, e in intervals)
        # Reject the entire buffer at window boundaries, not just individual stages.
        if first < start_ns or last > end_ns:
            excluded["outside_or_crossing_window_buffers"] += 1
            continue
        names = labels[identity]
        if not names:
            excluded["unmatched_buffers"] += 1
            continue
        if len(names) != 1:
            raise ValueError("Ambiguous command-buffer label")
        buffers.append({"command_buffer_id": identity, "label": next(iter(names)),
                        "start_ns": first, "end_ns": last,
                        "span_ms": (last - first) / 1e6,
                        "active_union_ms": union_ns(intervals) / 1e6,
                        "intervals": sorted(rows, key=lambda r: r["start_ns"])})
    if not buffers:
        raise ValueError("No matched target GPU buffers in analysis window")
    buffers.sort(key=lambda b: (b["start_ns"], b["command_buffer_id"]))
    summary = {}
    for label in sorted({b["label"] for b in buffers}):
        selected = [b for b in buffers if b["label"] == label]
        summary[label] = {metric: distribution([b[metric + "_ms"] for b in selected])
                          for metric in ["span", "active_union"]}
    for label in required_labels:
        if label not in summary:
            raise ValueError("Missing required GPU command-buffer label: " + label)
    return {"fixture": "native-metal-command-buffer-intervals-v1", "pid": pid,
            "window_ns": [start_ns, end_ns], "excluded": dict(excluded),
            "summary": summary, "buffers": buffers,
            "measurement": "Process-owned Active depth-0 GPU intervals joined to command-buffer Encoding metadata by numeric identity. Span includes gaps between GPU stages; active_union merges overlaps. Neither is full UI-frame time, CPU upload cost, or presentation latency. Unmatched and window-crossing buffers are excluded explicitly."}


def summarize_encoding(gpu_report, application):
    """Join encoding wall spans to accepted GPU buffers and same-thread drawable waits.

    A command buffer can remain open while its thread waits. Removing this known
    overlap does not turn the remainder into CPU time or isolate upload/staging.
    """
    pid = gpu_report["pid"]
    start_ns, end_ns = gpu_report["window_ns"]
    selected = {b["command_buffer_id"]: b for b in gpu_report["buffers"]}
    encodings = {}
    waits = []

    def thread_id(row):
        thread = row["thread"]
        owner = application.resolve(thread.find("process"))
        if number(application.resolve(owner.find("pid"))) != pid:
            raise ValueError("Thread does not belong to target process")
        return number(application.resolve(thread.find("tid")))

    for row in application.rows(pid):
        event = row["event-type"].text
        if event == "Wait for Next Drawable":
            start = number(row["start"])
            end = start + number(row["duration"])
            # Retain boundary-crossing waits in full, intersect them per buffer below.
            if start < end_ns and end > start_ns:
                waits.append({"thread_id": thread_id(row), "start_ns": start, "end_ns": end})
        elif event == "Encoding":
            identity = number(row["cmdbuffer-id"])
            label = row["event-label"].find("metal-object-label")
            if (identity not in selected or label is None
                    or row["event-label"].find("uint64") is None):
                continue  # Nested encoder rows are not command-buffer wall spans.
            if identity in encodings:
                raise ValueError("Duplicate command-buffer Encoding interval")
            if application.resolve(label).text != selected[identity]["label"]:
                raise ValueError("Encoding label does not match GPU buffer")
            start = number(row["start"])
            encodings[identity] = {"command_buffer_id": identity,
                                   "label": selected[identity]["label"],
                                   "thread_id": thread_id(row), "start_ns": start,
                                   "end_ns": start + number(row["duration"])}
    if encodings.keys() != selected.keys():
        raise ValueError("Missing command-buffer Encoding interval")

    buffers = []
    excluded = collections.Counter()
    for row in encodings.values():
        start, end = row["start_ns"], row["end_ns"]
        if start < start_ns or end > end_ns:
            excluded["outside_or_crossing_window_buffers"] += 1
            continue
        overlap = union_ns([(max(start, w["start_ns"]), min(end, w["end_ns"]))
                            for w in waits if w["thread_id"] == row["thread_id"]
                            and w["start_ns"] < end and w["end_ns"] > start])
        buffers.append({**row, "wall_ms": (end - start) / 1e6,
                        "drawable_wait_overlap_ms": overlap / 1e6,
                        "non_drawable_wait_wall_ms": (end - start - overlap) / 1e6})
    if not buffers:
        raise ValueError("No matched Encoding intervals in analysis window")
    buffers.sort(key=lambda b: (b["start_ns"], b["command_buffer_id"]))
    waits.sort(key=lambda w: (w["start_ns"], w["thread_id"], w["end_ns"]))
    summary = {}
    for label in sorted({b["label"] for b in buffers}):
        matching = [b for b in buffers if b["label"] == label]
        summary[label] = {metric: distribution([b[metric + "_ms"] for b in matching])
                          for metric in ["wall", "drawable_wait_overlap", "non_drawable_wait_wall"]}
    return {"fixture": "native-metal-encoding-wait-overlap-v1", "pid": pid,
            "window_ns": [start_ns, end_ns], "excluded": dict(excluded),
            "summary": summary, "buffers": buffers, "drawable_waits": waits,
            "measurement": "Command-buffer Encoding wall intervals joined by numeric identity to accepted GPU buffers. Same-process, same-thread Wait for Next Drawable intersections are unioned per buffer; nested encoder intervals are ignored. CPU encoding and GPU boundaries must both fit the fixed window. The remainder still includes unrelated CPU work, scheduling and other waits: it is not CPU time, upload/staging cost, or presentation latency. Per-buffer intervals can overlap and must not be summed as frame time."}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("gpu_xml")
    parser.add_argument("application_xml")
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--require-label", action="append", default=[])
    parser.add_argument("--include-encoding", action="store_true",
                        help="Include encoding wall / same-thread drawable-wait overlap, not CPU time")
    parser.add_argument("--start-ns", type=int, default=1_000_000_000)
    parser.add_argument("--end-ns", type=int, default=4_000_000_000)
    args = parser.parse_args()
    application = Table(args.application_xml, "metal-application-intervals")
    result = summarize(Table(args.gpu_xml, "metal-gpu-intervals"), application,
                       args.pid, args.start_ns, args.end_ns, args.require_label)
    if args.include_encoding:
        result["encoding"] = summarize_encoding(result, application)
    print(json.dumps(result, indent=2, allow_nan=False))


if __name__ == "__main__":
    main()
