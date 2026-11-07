#!/usr/bin/env python3
"""Rebuild src-tauri/assets/models_seed.json from a live OpenRouter catalog.

Idempotent: always regenerates the whole file from CURATED + the live $0 sweep.
Run:  python3 tools/build_seed.py [path/to/openrouter_models.json]

Pricing convention:
  >= 0  ->  real USD price
  -1    ->  variable / routed (cost depends on the model the router picks)
"""
import json
import os
import re
import sys
import urllib.request

OUT = os.path.join(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
    "src-tauri", "assets", "models_seed.json",
)

# (id, tier, badge) — hand-curated ordering and tiers.
CURATED = [
    ("anthropic/claude-opus-5", "Flagship", "pinned"),
    ("anthropic/claude-opus-5-fast", "Flagship", ""),
    ("anthropic/claude-fable-5", "Flagship", "pinned"),
    ("anthropic/claude-sonnet-5", "Balanced", "pinned"),
    ("anthropic/claude-haiku-4.5", "Fast", ""),
    ("anthropic/claude-opus-4.8", "Flagship", ""),
    ("openai/gpt-5.6-sol-pro", "Flagship", "pinned"),
    ("openai/gpt-5.6-sol", "Flagship", ""),
    ("openai/gpt-5.6-terra-pro", "Balanced", ""),
    ("openai/gpt-5.6-terra", "Balanced", ""),
    ("openai/gpt-5.6-luna", "Fast", "pinned"),
    ("openai/gpt-5.6-luna-pro", "Fast", ""),
    ("openai/gpt-5.5-pro", "Flagship", ""),
    ("openai/gpt-5.5", "Balanced", ""),
    ("openai/gpt-5.4-pro", "Flagship", ""),
    ("openai/gpt-5.4", "Balanced", ""),
    ("openai/gpt-5.4-mini", "Fast", ""),
    ("openai/gpt-5.4-nano", "Fast", ""),
    ("openai/gpt-5.3-codex", "Coding", ""),
    ("openai/gpt-5.2-codex", "Coding", ""),
    ("openai/o3-pro", "Reasoning", ""),
    ("openai/o4-mini-high", "Reasoning", ""),
    ("google/gemini-3.1-pro-preview", "Flagship", ""),
    ("google/gemini-3.6-flash", "Fast", ""),
    ("google/gemini-3.5-flash", "Fast", ""),
    ("google/gemini-3.5-flash-lite", "Fast", ""),
    ("google/gemini-3.1-flash-lite", "Fast", ""),
    ("google/gemini-2.5-pro", "Balanced", ""),
    ("google/gemini-2.5-flash", "Fast", ""),
    ("moonshotai/kimi-k3", "Flagship", "pinned"),
    ("moonshotai/kimi-k2.7-code", "Coding", ""),
    ("moonshotai/kimi-k2.6", "Balanced", ""),
    ("moonshotai/kimi-k2-thinking", "Reasoning", ""),
    ("deepseek/deepseek-v4-flash-0731", "Fast", "pinned"),
    ("deepseek/deepseek-v4-pro", "Flagship", ""),
    ("deepseek/deepseek-v4-flash", "Fast", ""),
    ("deepseek/deepseek-v3.2", "Balanced", ""),
    ("deepseek/deepseek-r1-0528", "Reasoning", ""),
    ("poolside/laguna-s-2.1", "Coding", "pinned"),
    ("poolside/laguna-xs-2.1", "Coding", ""),
    ("x-ai/grok-4.5", "Flagship", ""),
    ("x-ai/grok-4.20-multi-agent", "Agentic", ""),
    ("x-ai/grok-build-0.1", "Coding", ""),
    ("z-ai/glm-5.2", "Flagship", ""),
    ("z-ai/glm-5", "Balanced", ""),
    ("z-ai/glm-5-turbo", "Fast", ""),
    ("z-ai/glm-4.7", "Balanced", ""),
    ("qwen/qwen3.8-max", "Flagship", ""),
    ("qwen/qwen3.7-max", "Flagship", ""),
    ("qwen/qwen3-coder-next", "Coding", ""),
    ("qwen/qwen3-coder-plus", "Coding", ""),
    ("qwen/qwen3.7-flash", "Fast", ""),
    ("minimax/minimax-m3", "Balanced", ""),
    ("minimax/minimax-m2.7", "Balanced", ""),
    ("mistralai/mistral-large-2512", "Balanced", ""),
    ("mistralai/codestral-2508", "Coding", ""),
    ("mistralai/mistral-medium-3.1", "Balanced", ""),
    ("meta-llama/llama-4-maverick", "Balanced", ""),
    ("meta-llama/llama-4-scout", "Fast", ""),
    ("cohere/command-a", "Balanced", ""),
    ("upstage/solar-pro4", "Balanced", ""),
    ("nousresearch/hermes-3-llama-3.1-405b", "Creative", ""),
    # routers (variable cost)
    ("openrouter/auto", "Router", ""),
    ("openrouter/auto-beta", "Router", ""),
    ("openrouter/pareto-code", "Coding", ""),
    ("openrouter/fusion", "Agentic", ""),
    # image
    ("google/gemini-3-pro-image", "Image", ""),
    ("google/gemini-3.1-flash-image", "Image", ""),
    ("google/gemini-3.1-flash-lite-image", "Image", ""),
    ("google/gemini-2.5-flash-image", "Image", ""),
    ("openai/gpt-5.4-image-2", "Image", ""),
    ("openai/gpt-5-image", "Image", ""),
    ("openai/gpt-5-image-mini", "Image", ""),
    # audio
    ("openai/gpt-audio", "Audio", ""),
    ("openai/gpt-audio-mini", "Audio", ""),
    ("meta/muse-spark-1.2", "Audio", ""),
    ("mistralai/voxtral-small-24b-2507", "Audio", ""),
]

# The free auto-router is pinned to the very top of the Free group.
FREE_ROUTER = "openrouter/free"


def load(path=None):
    if path and os.path.exists(path):
        return json.load(open(path))["data"]
    url = "https://openrouter.ai/api/v1/models"
    with urllib.request.urlopen(url, timeout=60) as r:
        return json.load(r)["data"]


def raw_price(m, k):
    v = (m.get("pricing") or {}).get(k)
    try:
        return float(v)
    except (TypeError, ValueError):
        return 0.0


def is_variable(m):
    """Routers advertise -1: cost depends on which model they pick."""
    return raw_price(m, "prompt") < 0 or raw_price(m, "completion") < 0


def is_free(m):
    return (
        not is_variable(m)
        and raw_price(m, "prompt") == 0.0
        and raw_price(m, "completion") == 0.0
    )


def card(m, tier, badge):
    mid = m["id"]
    a = m.get("architecture") or {}
    sp = m.get("supported_parameters") or []
    desc = re.sub(r"\s+", " ", (m.get("description") or "").strip())
    if len(desc) > 240:
        desc = desc[:237].rsplit(" ", 1)[0] + "…"
    if is_variable(m):
        pin = pout = pimg = -1.0
    else:
        pin = raw_price(m, "prompt") * 1_000_000
        pout = raw_price(m, "completion") * 1_000_000
        pimg = raw_price(m, "image") * 1000
    return {
        "id": mid,
        "name": m.get("name") or mid,
        "vendor": mid.split("/")[0],
        "tier": tier,
        "badge": badge,
        "context": m.get("context_length") or 0,
        "max_out": (m.get("top_provider") or {}).get("max_completion_tokens") or 0,
        "in_mod": a.get("input_modalities") or ["text"],
        "out_mod": a.get("output_modalities") or ["text"],
        "price_in": pin,
        "price_out": pout,
        "price_img": pimg,
        "tools": "tools" in sp,
        "reasoning": "reasoning" in sp or "include_reasoning" in sp,
        "structured": "structured_outputs" in sp,
        "desc": desc,
    }


def main():
    data = load(sys.argv[1] if len(sys.argv) > 1 else None)
    by = {m["id"]: m for m in data}
    out, seen, missing = [], set(), []

    # 1. the free router leads
    if FREE_ROUTER in by:
        out.append(card(by[FREE_ROUTER], "Free", "free-router"))
        seen.add(FREE_ROUTER)
    else:
        missing.append(FREE_ROUTER)

    # 2. every genuinely $0 model, sorted by context descending (best first)
    frees = [
        m for m in data
        if not m["id"].startswith("~") and m["id"] not in seen and is_free(m)
    ]
    frees.sort(key=lambda m: -(m.get("context_length") or 0))
    for m in frees:
        om = (m.get("architecture") or {}).get("output_modalities") or ["text"]
        tier = "Audio" if "audio" in om else ("Image" if "image" in om else "Free")
        out.append(card(m, tier, "free"))
        seen.add(m["id"])

    # 3. the curated paid roster
    for mid, tier, badge in CURATED:
        if mid in seen:
            continue
        if mid not in by:
            missing.append(mid)
            continue
        out.append(card(by[mid], tier, badge))
        seen.add(mid)

    assert len(out) == len({c["id"] for c in out}), "duplicate ids in seed"
    with open(OUT, "w") as f:
        json.dump(out, f, indent=1)

    n_free = sum(1 for c in out if c["price_in"] == 0 and c["price_out"] == 0)
    n_var = sum(1 for c in out if c["price_out"] < 0)
    print(f"wrote {OUT}")
    print(f"  total={len(out)} free={n_free} variable/router={n_var}")
    print(f"  pinned={[c['id'] for c in out if c['badge'] == 'pinned']}")
    if missing:
        print(f"  MISSING (not in live catalog): {missing}")


if __name__ == "__main__":
    main()
