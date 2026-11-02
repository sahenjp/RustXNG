"""Structured URL-prefix matching for the legacy retrieval benchmarks.

Gold URLs describe pages or directory prefixes, not arbitrary string prefixes.
Scheme, fragments and version segments are ignored as in the legacy task.
Path case, encoded reserved delimiters, authority and non-default ports matter.
An extensionless reference also accepts its .html/.htm page. A reference query,
when present, must match exactly; a query-free directory reference permits any
query. These are task-specific rules, not the stricter SealQA page metric.
"""

from __future__ import annotations

import re
from urllib.parse import quote, urlsplit

_VERSION = re.compile(r"^(?:stable|current|latest|v?\d+(?:\.\d+)*)$")
_ESCAPE = re.compile(r"%([0-9a-fA-F]{2})")
_UNRESERVED = frozenset("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~")


def _path(value: str) -> str | None:
    def decode(match: re.Match) -> str:
        character = chr(int(match[1], 16))
        return character if character in _UNRESERVED else match[0].upper()

    value = quote(value, safe="/%:@!$&'()*+,;=-._~")
    value = _ESCAPE.sub(decode, value)
    if any(part in (".", "..") for part in value.split("/")):
        return None
    return "/".join("*" if _VERSION.fullmatch(part) else part for part in value.split("/")).rstrip("/")


def _parts(value: str) -> tuple[str, int | None, str, str] | None:
    try:
        parsed = urlsplit(value)
        if parsed.scheme.lower() not in ("http", "https") or not parsed.hostname:
            return None
        if parsed.username is not None or parsed.password is not None:
            return None
        port = parsed.port
        path = _path(parsed.path)
        if path is None:
            return None
        return parsed.hostname.lower(), None if port in (80, 443) else port, path, parsed.query
    except ValueError:
        return None


def matches_reference(candidate: str, reference: str) -> bool:
    found, gold = _parts(candidate), _parts(reference)
    if found is None or gold is None or found[:2] != gold[:2]:
        return False
    if gold[3] and found[3] != gold[3]:
        return False
    path, prefix = found[2], gold[2]
    return path == prefix or path.startswith(prefix + "/") or path in (prefix + ".html", prefix + ".htm")


def first_hit_rank(urls: list[str], references: list[str]) -> int | None:
    return next((index for index, url in enumerate(urls, 1)
                 if any(matches_reference(url, reference) for reference in references)), None)
