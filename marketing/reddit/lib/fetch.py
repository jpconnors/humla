#!/usr/bin/env python3
"""
Reddit JSON scraping helper for Humla marketing routines.

Replaces Reddit_MCP_Buddy after Reddit's policy change blocked the MCP's auth
path. Hits reddit.com's .json endpoints directly with a UA string. Practical
limit is ~60 requests/min per IP unauthenticated; the cache cuts that further.

Two ways to use it:

1. As a Python module (preferred from longer scripts):

       import sys
       sys.path.insert(0, "marketing/reddit/lib")
       from fetch import (
           browse_subreddit,
           search_subreddit,
           search_reddit,
           get_post_with_comments,
           walk_comments,
       )

2. As a CLI (preferred from a routine driven by an LLM, where tool output
   needs to be JSON on stdout):

       python3 marketing/reddit/lib/fetch.py browse AiNoteTaker --sort new --limit 25
       python3 marketing/reddit/lib/fetch.py search-sub AiNoteTaker "granola alternative" --time week
       python3 marketing/reddit/lib/fetch.py search "granola alternative" --time week
       python3 marketing/reddit/lib/fetch.py post AiNoteTaker 1t0022r
       python3 marketing/reddit/lib/fetch.py tree AiNoteTaker 1t0022r

   Cache: ~/.cache/humla-reddit/, 10-minute TTL. Pass --no-cache to bypass.
"""
from __future__ import annotations

import argparse
import json
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

UA = "humla-research/0.1 by u/tremendousquotes"
BASE = "https://www.reddit.com"
CACHE_DIR = Path.home() / ".cache" / "humla-reddit"
CACHE_TTL_SECONDS = 600  # 10 minutes
INITIAL_BACKOFF = 5
MAX_RETRIES = 4


def _cache_path(url: str) -> Path:
    return CACHE_DIR / urllib.parse.quote(url, safe="")


def _read_cache(url: str):
    p = _cache_path(url)
    if not p.exists():
        return None
    if time.time() - p.stat().st_mtime > CACHE_TTL_SECONDS:
        return None
    try:
        return json.loads(p.read_text())
    except json.JSONDecodeError:
        return None


def _write_cache(url: str, data) -> None:
    CACHE_DIR.mkdir(parents=True, exist_ok=True)
    _cache_path(url).write_text(json.dumps(data))


def _get(url: str, *, use_cache: bool = True):
    if use_cache:
        cached = _read_cache(url)
        if cached is not None:
            return cached
    backoff = INITIAL_BACKOFF
    last_err: Exception | None = None
    for attempt in range(MAX_RETRIES):
        req = urllib.request.Request(url, headers={"User-Agent": UA})
        try:
            with urllib.request.urlopen(req, timeout=20) as resp:
                data = json.loads(resp.read())
                _write_cache(url, data)
                return data
        except urllib.error.HTTPError as e:
            last_err = e
            if e.code in (429, 500, 502, 503, 504) and attempt + 1 < MAX_RETRIES:
                time.sleep(backoff)
                backoff *= 2
                continue
            raise
        except urllib.error.URLError as e:
            last_err = e
            if attempt + 1 < MAX_RETRIES:
                time.sleep(backoff)
                backoff *= 2
                continue
            raise
    raise RuntimeError(f"max retries exceeded: {url} ({last_err})")


def _flatten_listing(data) -> list[dict]:
    """Reddit listings nest posts in data.children[].data; t3 = link/post."""
    children = data.get("data", {}).get("children", [])
    return [c["data"] for c in children if c.get("kind") == "t3"]


def browse_subreddit(
    sub: str,
    *,
    sort: str = "new",
    time_range: str = "day",
    limit: int = 25,
    use_cache: bool = True,
) -> list[dict]:
    """Equivalent of Reddit_MCP_Buddy browse_subreddit."""
    if sort in ("top", "controversial"):
        url = f"{BASE}/r/{sub}/{sort}.json?t={time_range}&limit={limit}"
    elif sort in ("hot", "new", "rising"):
        url = f"{BASE}/r/{sub}/{sort}.json?limit={limit}"
    else:
        raise ValueError(f"unknown sort: {sort}")
    return _flatten_listing(_get(url, use_cache=use_cache))


def search_subreddit(
    sub: str,
    query: str,
    *,
    sort: str = "new",
    time_range: str = "week",
    limit: int = 25,
    use_cache: bool = True,
) -> list[dict]:
    """Search inside one sub via restrict_sr=1."""
    q = urllib.parse.quote(query)
    url = (
        f"{BASE}/r/{sub}/search.json"
        f"?q={q}&restrict_sr=1&sort={sort}&t={time_range}&limit={limit}"
    )
    return _flatten_listing(_get(url, use_cache=use_cache))


def search_reddit(
    query: str,
    *,
    sort: str = "new",
    time_range: str = "week",
    limit: int = 25,
    use_cache: bool = True,
) -> list[dict]:
    """Reddit-wide search."""
    q = urllib.parse.quote(query)
    url = f"{BASE}/search.json?q={q}&sort={sort}&t={time_range}&limit={limit}"
    return _flatten_listing(_get(url, use_cache=use_cache))


def get_post_with_comments(
    sub: str,
    post_id: str,
    *,
    depth: int = 10,
    limit: int = 200,
    use_cache: bool = True,
) -> dict:
    """Fetch a post + its full comment tree.

    Returns {"post": <post dict>, "comments_raw": [<children list>]}.
    Comments are raw Reddit children (kind: t1 or more), unflattened. Use
    walk_comments() to get a flat list with depth + reply counts.
    """
    url = f"{BASE}/r/{sub}/comments/{post_id}.json?depth={depth}&limit={limit}"
    raw = _get(url, use_cache=use_cache)
    post = raw[0]["data"]["children"][0]["data"]
    comments = raw[1]["data"]["children"]
    return {"post": post, "comments_raw": comments}


def walk_comments(post_data: dict, *, max_depth: int = 50) -> list[dict]:
    """Flatten a post_data["comments_raw"] tree into a list of dicts:

       {id, author, score, body, depth, parent_id, num_replies}.

    num_replies counts direct children only. Use depth to reconstruct nesting.
    """
    out: list[dict] = []

    def _walk(c, depth):
        if depth > max_depth or c.get("kind") != "t1":
            return
        d = c.get("data", {})
        replies = d.get("replies")
        reply_children = []
        if isinstance(replies, dict):
            reply_children = replies.get("data", {}).get("children", [])
        out.append(
            {
                "id": d.get("id"),
                "author": d.get("author"),
                "score": d.get("score"),
                "body": d.get("body", ""),
                "depth": depth,
                "parent_id": d.get("parent_id"),
                "num_replies": len(reply_children),
            }
        )
        for child in reply_children:
            _walk(child, depth + 1)

    for c in post_data["comments_raw"]:
        _walk(c, 0)
    return out


def _print_json(obj) -> None:
    json.dump(obj, sys.stdout, ensure_ascii=False, indent=2)
    sys.stdout.write("\n")


def main(argv: list[str] | None = None) -> int:
    # --no-cache is shared across all subcommands so it works whether the
    # user puts it before or after the subcommand name.
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument("--no-cache", action="store_true", help="bypass on-disk cache")

    p = argparse.ArgumentParser(prog="reddit-fetch", parents=[common])
    sub = p.add_subparsers(dest="cmd", required=True)

    b = sub.add_parser("browse", parents=[common], help="list posts from a sub")
    b.add_argument("subreddit")
    b.add_argument("--sort", default="new", choices=["hot", "new", "rising", "top", "controversial"])
    b.add_argument("--time", dest="time_range", default="day", choices=["hour", "day", "week", "month", "year", "all"])
    b.add_argument("--limit", type=int, default=25)

    ss = sub.add_parser("search-sub", parents=[common], help="keyword search inside one sub")
    ss.add_argument("subreddit")
    ss.add_argument("query")
    ss.add_argument("--sort", default="new", choices=["relevance", "hot", "new", "top", "comments"])
    ss.add_argument("--time", dest="time_range", default="week", choices=["hour", "day", "week", "month", "year", "all"])
    ss.add_argument("--limit", type=int, default=25)

    sr = sub.add_parser("search", parents=[common], help="Reddit-wide keyword search")
    sr.add_argument("query")
    sr.add_argument("--sort", default="new", choices=["relevance", "hot", "new", "top", "comments"])
    sr.add_argument("--time", dest="time_range", default="week", choices=["hour", "day", "week", "month", "year", "all"])
    sr.add_argument("--limit", type=int, default=25)

    pp = sub.add_parser("post", parents=[common], help="fetch a single post (no comments)")
    pp.add_argument("subreddit")
    pp.add_argument("post_id")

    pt = sub.add_parser("tree", parents=[common], help="fetch a post and walk the full comment tree")
    pt.add_argument("subreddit")
    pt.add_argument("post_id")
    pt.add_argument("--depth", type=int, default=10)
    pt.add_argument("--limit", type=int, default=200)
    pt.add_argument("--print", dest="print_tree", action="store_true", help="print indented human-readable tree instead of JSON")

    args = p.parse_args(argv)
    use_cache = not args.no_cache

    if args.cmd == "browse":
        _print_json(browse_subreddit(args.subreddit, sort=args.sort, time_range=args.time_range, limit=args.limit, use_cache=use_cache))
    elif args.cmd == "search-sub":
        _print_json(search_subreddit(args.subreddit, args.query, sort=args.sort, time_range=args.time_range, limit=args.limit, use_cache=use_cache))
    elif args.cmd == "search":
        _print_json(search_reddit(args.query, sort=args.sort, time_range=args.time_range, limit=args.limit, use_cache=use_cache))
    elif args.cmd == "post":
        full = get_post_with_comments(args.subreddit, args.post_id, use_cache=use_cache)
        _print_json(full["post"])
    elif args.cmd == "tree":
        full = get_post_with_comments(args.subreddit, args.post_id, depth=args.depth, limit=args.limit, use_cache=use_cache)
        if args.print_tree:
            for c in walk_comments(full):
                indent = "  " * c["depth"]
                body = c["body"].replace("\n", " ")[:200]
                sys.stdout.write(f'{indent}- [{c["id"]}] u/{c["author"]} [{c["score"]}↑]: {body}\n')
        else:
            _print_json(walk_comments(full))
    return 0


if __name__ == "__main__":
    sys.exit(main())
