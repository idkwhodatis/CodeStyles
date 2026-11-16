// CodeStyles formatter-only adapter. MIT licensed; not a Caddy server.
package main

import (
	"bytes"
	_ "embed"
	"encoding/json"
	"unicode/utf8"

	"codestyles-caddyfmt/internal/caddyfile"
)

const limit = 32 * 1024 * 1024

var shared, formatted []byte
var lastError, overrideError string
var configs = map[uint32]string{}

//go:embed licenses/Caddy-Apache-2.0.txt
var caddyLicense string

//go:embed licenses/Go-BSD-3-Clause.txt
var goLicense string

//go:embed licenses/TinyGo-BSD-3-Clause.txt
var tinygoLicense string

//go:embed licenses/LLVM-compiler-rt.txt
var llvmLicense string

//go:embed LICENSE
var adapterLicense string

func setShared(data []byte) uint32 {
	shared = data
	return uint32(len(data))
}

func takeShared() []byte {
	data := shared
	shared = nil
	return data
}

func emptySettings(data []byte) string {
	if len(bytes.TrimSpace(data)) == 0 {
		return ""
	}
	var config map[string]json.RawMessage
	if json.Unmarshal(data, &config) != nil || config == nil {
		return "Caddy formatter settings must be an empty JSON object."
	}
	if len(config) != 0 {
		return "Caddy formatter has fixed upstream formatting and accepts no style options."
	}
	return ""
}

func registerConfig(id uint32) {
	var config map[string]json.RawMessage
	if json.Unmarshal(takeShared(), &config) != nil || config == nil {
		configs[id] = "Invalid dprint configuration envelope."
		return
	}
	// Global style options are intentionally ignored: Caddy has fixed formatting.
	plugin := config["plugin"]
	if len(plugin) == 0 {
		plugin = []byte("{}")
	}
	configs[id] = emptySettings(plugin)
}

func diagnostics(id uint32) uint32 {
	message := configs[id]
	if message == "" {
		return setShared([]byte("[]"))
	}
	data, _ := json.Marshal([]map[string]string{{"propertyName": "caddyfmt", "message": message}})
	return setShared(data)
}

func fail(message string) uint32 {
	formatted = nil
	lastError = message
	return 2
}

func formatText(id uint32) uint32 {
	input := takeShared()
	formatted = nil
	lastError = ""
	currentOverrideError := overrideError
	overrideError = ""
	if message := configs[id]; message != "" {
		return fail(message)
	}
	if currentOverrideError != "" {
		return fail(currentOverrideError)
	}
	if len(input) > limit {
		return fail("Caddyfile input exceeds 32 MiB.")
	}
	if !utf8.Valid(input) {
		return fail("Caddyfile input must be UTF-8.")
	}
	output := caddyfile.Format(input)
	if len(output) > limit {
		return fail("Caddyfile output exceeds 32 MiB.")
	}
	if bytes.Equal(input, output) {
		return 0
	}
	formatted = output
	return 1
}
