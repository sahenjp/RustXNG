#!/usr/bin/env python3
"""Check Standard defaults; a documented pending review is not permission approval."""
from __future__ import annotations

import json
import tomllib
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def identity(engine: dict) -> dict:
    return {
        "module": engine.get("engine", engine["name"]),
        "base_url": engine.get("base_url", ""),
        "search_url": engine.get("search_url", ""),
    }


def spacing_for(outgoing: dict, host: str) -> int:
    matches = [
        (domain, millis)
        for key, millis in outgoing.get("host_spacing_overrides", {}).items()
        if (domain := key.strip().lstrip(".").lower())
        and (host == domain or host.endswith("." + domain))
    ]
    return max(matches, key=lambda item: len(item[0]))[1] if matches else outgoing.get("host_spacing_ms", 0)


def validate(settings: dict, policy: dict, pending: dict) -> list[str]:
    errors = []
    for engine in settings["engines"]:
        if engine.get("disabled", False) or engine.get("inactive", False):
            continue
        name = engine["name"]
        module = engine.get("engine", name)
        review = policy["providers"].get(module, {})
        allowed = review.get("standard_allowed")
        if allowed is False:
            errors.append(f"{name}: {module} must be disabled in Standard ({review.get('decision')})")
        elif allowed is True:
            for field in ("base_url", "search_url"):
                if engine.get(field, "") != review.get(field, ""):
                    errors.append(f"{name}: configured {field} needs a separate endpoint review")
            for field in ("terms_url", "request_url", "access_method", "status", "decision", "conditions"):
                if not review.get(field):
                    errors.append(f"{name}: reviewed provider needs {field}")
            try:
                if date.fromisoformat(review.get("review_date", "")) > date.today():
                    raise ValueError("future review date")
            except (TypeError, ValueError):
                errors.append(f"{name}: valid non-future review_date required")
            if minimum := review.get("minimum_spacing_ms"):
                host = review["rate_limit_host"]
                if spacing_for(settings.get("outgoing", {}), host) < minimum:
                    errors.append(f"{name}: {host} requires at least {minimum} ms between requests")
        elif pending.get(name) != identity(engine):
            errors.append(f"{name}: new or changed Standard provider needs a permission review")
    return errors


def main() -> None:
    settings = tomllib.loads((ROOT / "settings.toml").read_text(encoding="utf-8"))
    policy = json.loads((ROOT / "docs/provider-policy.json").read_text(encoding="utf-8"))
    pending = json.loads((ROOT / "docs/provider-review-pending.json").read_text(encoding="utf-8"))["engines"]
    errors = validate(settings, policy, pending)
    if errors:
        raise SystemExit("\n".join(errors))
    print("Standard policy checks passed. Pending entries remain unreviewed; see docs/PROVIDERS.md.")


if __name__ == "__main__":
    main()
