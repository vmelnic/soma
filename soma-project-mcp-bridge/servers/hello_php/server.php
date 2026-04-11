#!/usr/bin/env php
<?php
// Minimal MCP stdio server exposing greet + reverse tools.
//
// Pure PHP stdlib — no Composer dependency so this proof project works on
// any PHP 8+ install. The greet response includes "(from php)" so the
// smoke test can tell the three language servers apart side by side.

const PROTOCOL_VERSION = "2024-11-05";

$TOOLS = [
    [
        "name" => "greet",
        "description" => "Return a personalized greeting for the given name.",
        "inputSchema" => [
            "type" => "object",
            "required" => ["name"],
            "properties" => [
                "name" => ["type" => "string", "description" => "Who to greet."],
            ],
        ],
    ],
    [
        "name" => "reverse",
        "description" => "Reverse the given text.",
        "inputSchema" => [
            "type" => "object",
            "required" => ["text"],
            "properties" => [
                "text" => ["type" => "string"],
            ],
        ],
    ],
];

function respond($id, $result = null, $error = null): void
{
    $msg = ["jsonrpc" => "2.0", "id" => $id];
    if ($error !== null) {
        $msg["error"] = $error;
    } else {
        $msg["result"] = $result;
    }
    fwrite(STDOUT, json_encode($msg, JSON_UNESCAPED_SLASHES) . "\n");
    fflush(STDOUT);
}

function handle(array $req, array $tools): void
{
    $method = $req["method"] ?? "";
    $id = $req["id"] ?? null;
    $params = $req["params"] ?? [];

    if ($method === "initialize") {
        respond($id, [
            "protocolVersion" => PROTOCOL_VERSION,
            // Force an empty object — otherwise json_encode([]) emits [].
            "capabilities" => ["tools" => (object) []],
            "serverInfo" => ["name" => "hello-mcp-php", "version" => "0.1.0"],
        ]);
        return;
    }

    if ($method === "notifications/initialized") {
        return;
    }

    if ($method === "tools/list") {
        respond($id, ["tools" => $tools]);
        return;
    }

    if ($method === "tools/call") {
        $name = $params["name"] ?? null;
        $args = $params["arguments"] ?? [];

        if ($name === "greet") {
            $who = $args["name"] ?? "stranger";
            $payload = ["message" => "hello {$who}! (from php)"];
            respond($id, [
                "content" => [["type" => "text", "text" => json_encode($payload, JSON_UNESCAPED_SLASHES)]],
                "isError" => false,
            ]);
            return;
        }

        if ($name === "reverse") {
            $text = $args["text"] ?? "";
            $payload = ["reversed" => strrev($text)];
            respond($id, [
                "content" => [["type" => "text", "text" => json_encode($payload, JSON_UNESCAPED_SLASHES)]],
                "isError" => false,
            ]);
            return;
        }

        respond($id, null, ["code" => -32601, "message" => "unknown tool: {$name}"]);
        return;
    }

    if ($id !== null) {
        respond($id, null, ["code" => -32601, "message" => "unknown method: {$method}"]);
    }
}

while (($line = fgets(STDIN)) !== false) {
    $line = trim($line);
    if ($line === "") {
        continue;
    }
    $req = json_decode($line, true);
    if ($req === null) {
        fwrite(STDERR, "[hello-mcp-php] invalid JSON\n");
        continue;
    }
    handle($req, $TOOLS);
}
