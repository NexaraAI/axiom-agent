#!/usr/bin/env python3
"""Extract a contiguous line range from a Rust file into a new module.

Encoding-safe: reads/writes UTF-8 explicitly and preserves the file's
existing newline convention. Intended for behavior-preserving refactors
where a block moves to its own module verbatim.

Usage:
    python extract_block.py <src> <start> <end> <dst> [--prepend NAME] [--append-toml]
    python extract_block.py --audit <src> <start> <end>
"""
import argparse
import io
import os
import sys


def read_lines(path):
    with io.open(path, "r", encoding="utf-8", newline="") as handle:
        text = handle.read()
    newline = "\r\n" if "\r\n" in text else "\n"
    return text.splitlines(keepends=True), newline


def write_lines(path, lines, newline):
    with io.open(path, "w", encoding="utf-8", newline="") as handle:
        handle.writelines(lines)


def non_ascii(lines):
    counts = {}
    for line in lines:
        for ch in line:
            if ord(ch) > 127:
                counts[ord(ch)] = counts.get(ord(ch), 0) + 1
    return counts


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("src")
    parser.add_argument("start", type=int)
    parser.add_argument("end", type=int)
    parser.add_argument("dst", nargs="?")
    parser.add_argument("--audit", action="store_true")
    parser.add_argument("--prepend", default=None,
                        help="text file of header lines to place above the block")
    args = parser.parse_args()

    lines, newline = read_lines(args.src)
    start, end = args.start - 1, args.end  # 1-indexed inclusive -> 0-indexed slice

    if start < 0 or end > len(lines) or start >= end:
        sys.exit("bad range %d..%d (file has %d lines)" % (args.start, args.end, len(lines)))

    block = lines[start:end]
    before = sum(non_ascii(lines[:start]).values())
    inside = sum(non_ascii(block).values())
    after = sum(non_ascii(lines[end:]).values())

    if args.audit:
        print("file lines: %d" % len(lines))
        print("extract %d..%d (%d lines)" % (args.start, args.end, len(block)))
        print("non-ascii before/inside/after: %d/%d/%d" % (before, inside, after))
        return

    header = []
    if args.prepend:
        with io.open(args.prepend, "r", encoding="utf-8", newline="") as handle:
            header = handle.read().splitlines(keepends=True)

    os.makedirs(os.path.dirname(args.dst) or ".", exist_ok=True)
    write_lines(args.dst, header + block, newline)

    del lines[start:end]
    write_lines(args.src, lines, newline)

    remaining = sum(non_ascii(lines).values())
    print("wrote %s (%d lines)" % (args.dst, len(header) + len(block)))
    print("src now %d lines" % len(lines))
    print("non-ascii: extracted %d, remaining %d, total %d (was %d)"
          % (inside, remaining, inside + remaining, before + inside + after))
    if inside + remaining != before + inside + after:
        sys.exit("MISMATCH")


if __name__ == "__main__":
    main()
