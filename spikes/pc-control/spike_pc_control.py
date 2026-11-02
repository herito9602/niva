# /// script
# requires-python = ">=3.11,<3.13"
# dependencies = [
#   "anthropic",
#   "ollama",
#   "psutil",
#   "pycaw",
# ]
# ///
"""Throwaway spike: validate PC control through Claude tool use for Niva.

Typed command -> the LLM picks tools -> the app runs them -> short spoken-style reply.
Risky tools (closing apps, power actions) ask for confirmation before running.
Measures per-request and end-to-end latency.

The LLM sits behind a small provider port, so Ollama (local, free) and Claude
(cloud API) are interchangeable.

Usage:
  uv run spike_pc_control.py --dry-run                    # Ollama, print actions only
  uv run spike_pc_control.py --model qwen3.5:2b           # Ollama, real actions
  uv run spike_pc_control.py --provider claude --dry-run  # needs ANTHROPIC_API_KEY
  uv run spike_pc_control.py --selftest                   # local tools only, no LLM
"""

import argparse
import difflib
import json
import subprocess
import time
import urllib.parse
import webbrowser
from dataclasses import dataclass

import psutil

SYSTEM_PROMPT = """You are Niva, a voice assistant that controls the user's Windows PC.
Use the tools to carry out what the user asks. Your reply is spoken aloud: answer in the
user's language, in one short sentence, without markdown.
Only do what the user asked: once the requested actions are done, stop calling tools and reply.
Never lock, sleep, restart or shut down the PC unless the user explicitly asked for it.
If an app name is ambiguous or not found, use the candidates the tool returns instead of guessing.
Closing apps and power actions ask the user for confirmation inside the app; you don't need to ask first."""

TOOLS = [
    {
        "name": "open_app",
        "description": "Open an installed application by its name, e.g. 'Spotify', 'Calculator', 'Google Chrome'.",
        "input_schema": {
            "type": "object",
            "properties": {"name": {"type": "string", "description": "Application name as the user said it."}},
            "required": ["name"],
            "additionalProperties": False,
        },
    },
    {
        "name": "close_app",
        "description": "Close a running application by its name. Asks the user for confirmation.",
        "input_schema": {
            "type": "object",
            "properties": {"name": {"type": "string"}},
            "required": ["name"],
            "additionalProperties": False,
        },
    },
    {
        "name": "set_volume",
        "description": "Set the master volume to a percentage from 0 to 100, or mute/unmute.",
        "input_schema": {
            "type": "object",
            "properties": {
                "level": {"type": ["integer", "null"], "description": "0-100, or null to only change mute."},
                "mute": {"type": ["boolean", "null"], "description": "true to mute, false to unmute, null to leave as is."},
            },
            "required": ["level", "mute"],
            "additionalProperties": False,
        },
    },
    {
        "name": "web_search",
        "description": "Open a web search in the default browser.",
        "input_schema": {
            "type": "object",
            "properties": {"query": {"type": "string"}},
            "required": ["query"],
            "additionalProperties": False,
        },
    },
    {
        "name": "power_action",
        "description": "Lock, sleep, restart or shut down the PC. Asks the user for confirmation.",
        "input_schema": {
            "type": "object",
            "properties": {"action": {"type": "string", "enum": ["lock", "sleep", "restart", "shutdown"]}},
            "required": ["action"],
            "additionalProperties": False,
        },
    },
]


class ToolError(Exception):
    pass


def load_start_apps():
    """Every app in the Start menu, including Store apps, as {name: AppID}."""
    out = subprocess.run(
        # Force UTF-8: the console's OEM code page mangles accented app names.
        ["powershell", "-NoProfile", "-Command",
         "[Console]::OutputEncoding = [Text.Encoding]::UTF8; Get-StartApps | ConvertTo-Json -Compress"],
        capture_output=True, text=True, encoding="utf-8",
    ).stdout
    return {app["Name"]: app["AppID"] for app in json.loads(out or "[]")}


def best_matches(query, names, n=5):
    query_l = query.lower()
    contains = [name for name in names if query_l in name.lower()]
    close = difflib.get_close_matches(query, names, n=n, cutoff=0.5)
    return list(dict.fromkeys(sorted(contains, key=len) + close))[:n]


def confirm(question):
    try:
        answer = input(f"  [confirm] {question} (y/n) ")
    except EOFError:  # non-interactive run: never auto-approve
        print("n")
        return False
    return answer.strip().lower() in ("y", "s", "yes", "si", "sí")


class PcTools:
    def __init__(self, dry_run):
        self.dry_run = dry_run
        self.apps = load_start_apps()

    def act(self, description, action):
        # The model always sees success, so dry runs don't make it retry the action.
        if self.dry_run:
            print(f"    (dry run, skipped: {description})")
        else:
            action()
        return f"done: {description}"

    def open_app(self, name):
        matches = best_matches(name, list(self.apps))
        if not matches:
            raise ToolError(f"No installed app matches '{name}'.")
        exact = [m for m in matches if m.lower() == name.lower()]
        if not exact and len(matches) > 1 and name.lower() not in matches[0].lower():
            raise ToolError(f"Ambiguous app name '{name}'. Candidates: {matches}")
        app = exact[0] if exact else matches[0]
        app_id = self.apps[app]
        return self.act(f"open {app}", lambda: subprocess.Popen(["explorer.exe", f"shell:AppsFolder\\{app_id}"]))

    def close_app(self, name):
        running = {p.info["name"] for p in psutil.process_iter(["name"]) if p.info["name"]}
        stems = {n.removesuffix(".exe"): n for n in running}
        # Substring match only: fuzzy matching once mapped "Spotify" to FortiTray.exe.
        query = name.replace(" ", "").lower()
        matches = sorted((s for s in stems if query in s.lower()), key=len)
        if not matches:
            raise ToolError(f"No running process matches '{name}'.")
        exe = stems[matches[0]]
        if not confirm(f"Close {exe}? Unsaved work may be lost."):
            return "The user declined."
        # taskkill without /F asks the app to close gracefully.
        return self.act(f"close {exe}", lambda: subprocess.run(["taskkill", "/IM", exe], capture_output=True))

    def set_volume(self, level, mute):
        from pycaw.pycaw import AudioUtilities

        volume = AudioUtilities.GetSpeakers().EndpointVolume
        parts = []
        if level is not None:
            level = max(0, min(100, level))
            parts.append(f"volume {level}%")
        if mute is not None:
            parts.append("mute" if mute else "unmute")

        def apply():
            if level is not None:
                volume.SetMasterVolumeLevelScalar(level / 100, None)
            if mute is not None:
                volume.SetMute(int(mute), None)

        return self.act("set " + ", ".join(parts), apply)

    def web_search(self, query):
        url = "https://www.google.com/search?q=" + urllib.parse.quote(query)
        return self.act(f"search '{query}'", lambda: webbrowser.open(url))

    def power_action(self, action):
        commands = {
            "lock": ["rundll32.exe", "user32.dll,LockWorkStation"],
            "sleep": ["rundll32.exe", "powrprof.dll,SetSuspendState", "0,1,0"],
            "restart": ["shutdown", "/r", "/t", "5"],
            "shutdown": ["shutdown", "/s", "/t", "5"],
        }
        if not confirm(f"{action.capitalize()} the PC?"):
            return "The user declined."
        return self.act(action, lambda: subprocess.run(commands[action]))

    def run(self, name, args):
        return getattr(self, name)(**args)


@dataclass
class ToolCall:
    id: str
    name: str
    args: dict


@dataclass
class ToolResult:
    call: ToolCall
    content: str
    is_error: bool


@dataclass
class Turn:
    text: str
    tool_calls: list[ToolCall]


class ClaudeProvider:
    """Claude API adapter. Keeps the conversation in Anthropic's message format."""

    def __init__(self, model, effort):
        import anthropic

        self.client = anthropic.Anthropic()
        self.model = model
        self.effort = effort
        self.tools = [{**tool, "strict": True} for tool in TOOLS]
        self.messages = []

    def start(self, text):
        self.messages = [{"role": "user", "content": text}]
        return self._request()

    def send_results(self, results):
        self.messages.append({
            "role": "user",
            "content": [
                {"type": "tool_result", "tool_use_id": r.call.id, "content": r.content, "is_error": r.is_error}
                for r in results
            ],
        })
        return self._request()

    def _request(self):
        response = self.client.beta.messages.create(
            model=self.model,
            max_tokens=2048,
            system=SYSTEM_PROMPT,
            tools=self.tools,
            messages=self.messages,
            output_config={"effort": self.effort},
            betas=["server-side-fallback-2026-07-01"],
            fallbacks="default",
        )
        if response.stop_reason == "refusal":
            return Turn("(request declined by the model)", [])
        self.messages.append({"role": "assistant", "content": response.content})
        calls = [ToolCall(b.id, b.name, b.input) for b in response.content if b.type == "tool_use"]
        text = " ".join(b.text for b in response.content if b.type == "text")
        return Turn(text, calls)


class OllamaProvider:
    """Local Ollama adapter. Tool schemas use the OpenAI-style function wrapper."""

    def __init__(self, model, cpu_only=False):
        import ollama

        self.client = ollama.Client()
        self.model = model
        self.options = {"num_gpu": 0} if cpu_only else None
        self.tools = [
            {"type": "function", "function": {"name": t["name"], "description": t["description"], "parameters": t["input_schema"]}}
            for t in TOOLS
        ]
        self.messages = []

    def start(self, text):
        self.messages = [{"role": "system", "content": SYSTEM_PROMPT}, {"role": "user", "content": text}]
        return self._request()

    def send_results(self, results):
        for r in results:
            self.messages.append({"role": "tool", "tool_name": r.call.name, "content": r.content})
        return self._request()

    def _request(self):
        response = self.client.chat(
            model=self.model, messages=self.messages, tools=self.tools, think=False, options=self.options
        )
        message = response.message
        self.messages.append(message)
        calls = [
            ToolCall(f"call_{i}", c.function.name, dict(c.function.arguments))
            for i, c in enumerate(message.tool_calls or [])
        ]
        return Turn(message.content or "", calls)


def handle(provider, tools, text):
    started = time.perf_counter()
    t0 = time.perf_counter()
    turn = provider.start(text)
    for step in range(1, 7):
        print(f"  llm #{step}: {time.perf_counter() - t0:.2f}s  tool calls={len(turn.tool_calls)}")
        if not turn.tool_calls:
            print(f"  niva: {turn.text.strip()}")
            break
        results = []
        for call in turn.tool_calls:
            print(f"  tool {call.name}({json.dumps(call.args, ensure_ascii=False)})")
            try:
                result, is_error = tools.run(call.name, call.args), False
            except (ToolError, TypeError, AttributeError) as exc:
                # TypeError/AttributeError: the model invented a tool or arguments.
                result, is_error = f"Error: {exc}", True
            print(f"    -> {result}")
            results.append(ToolResult(call, result, is_error))
        t0 = time.perf_counter()
        turn = provider.send_results(results)
    print(f"  total: {time.perf_counter() - started:.2f}s\n")


def selftest():
    t0 = time.perf_counter()
    tools = PcTools(dry_run=True)
    print(f"Start menu apps: {len(tools.apps)} loaded in {time.perf_counter() - t0:.2f}s")
    for name in ["Spotify", "calculadora", "chrome", "notepad", "xyz-not-an-app"]:
        try:
            print(f"open_app({name!r}) -> {tools.open_app(name)}")
        except ToolError as exc:
            print(f"open_app({name!r}) -> error: {exc}")
    print(f"set_volume(40, None) -> {tools.set_volume(40, None)}")
    print(f"web_search -> {tools.web_search('clima en Buenos Aires')}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--provider", default="ollama", choices=["ollama", "claude"])
    parser.add_argument("--model", help="defaults: qwen3.5:2b (ollama), claude-opus-5-5 (claude)")
    parser.add_argument("--effort", default="low", choices=["low", "medium", "high"], help="claude only")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--cpu", action="store_true", help="ollama only: run the model without the GPU")
    parser.add_argument("--selftest", action="store_true")
    parser.add_argument("command", nargs="*", help="run these commands and exit instead of prompting")
    args = parser.parse_args()
    if args.selftest:
        return selftest()

    if args.provider == "claude":
        provider = ClaudeProvider(args.model or "claude-opus-5-5", args.effort)
    else:
        provider = OllamaProvider(args.model or "qwen3.5:2b", cpu_only=args.cpu)
    tools = PcTools(dry_run=args.dry_run)
    print(f"Niva PC control - {args.provider}:{provider.model}{', DRY RUN' if args.dry_run else ''}")
    if args.command:
        for text in args.command:
            print(f"you: {text}")
            handle(provider, tools, text)
        return
    print("Type a command (empty line to quit).\n")
    while text := input("you: ").strip():
        handle(provider, tools, text)


if __name__ == "__main__":
    main()
