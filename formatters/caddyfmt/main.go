//go:build tinygo

// CodeStyles dprint schema-v4 ABI adapter. MIT licensed.
// ABI reference: dprint 0.63.0 docs/wasm-plugin-development.md.
package main

import "unsafe"

func main() {}

//go:wasmexport get_shared_bytes_ptr
func getSharedBytesPtr() uint32 {
	if len(shared) == 0 {
		return 0
	}
	return uint32(uintptr(unsafe.Pointer(&shared[0])))
}

//go:wasmexport clear_shared_bytes
func clearSharedBytes(size uint32) uint32 {
	// A hostile or broken host must not allocate an unbounded guest buffer.
	// One extra byte allows input-limit errors to be returned without a trap.
	if size > limit+1 {
		panic("shared transfer exceeds 32 MiB plus one byte")
	}
	shared = make([]byte, size)
	return getSharedBytesPtr()
}

//go:wasmexport dprint_plugin_version_4
func pluginVersion() uint32 { return 4 }

//go:wasmexport get_plugin_info
func pluginInfo() uint32 {
	return setShared([]byte(`{"name":"codestyles-caddyfmt","version":"2.11.7-p1","configKey":"caddyfmt","helpUrl":"https://caddyserver.com/docs/command-line#caddy-fmt","configSchemaUrl":""}`))
}

//go:wasmexport get_license_text
func licenseText() uint32 {
	return setShared([]byte("Unpublished formatter candidate; complete distribution license review remains required.\n\nCaddy:\n" + caddyLicense + "\nGo:\n" + goLicense + "\nTinyGo:\n" + tinygoLicense + "\nLLVM compiler-rt:\n" + llvmLicense + "\nCodeStyles adapter:\n" + adapterLicense))
}

//go:wasmexport register_config
func register(id uint32) { registerConfig(id) }

//go:wasmexport release_config
func release(id uint32) { delete(configs, id) }

//go:wasmexport get_config_diagnostics
func getDiagnostics(id uint32) uint32 { return diagnostics(id) }

//go:wasmexport get_resolved_config
func resolvedConfig(id uint32) uint32 { return setShared([]byte("{}")) }

//go:wasmexport get_config_file_matching
func configFileMatching(id uint32) uint32 {
	return setShared([]byte(`{"fileExtensions":["caddyfile"],"fileNames":["Caddyfile"],"additive":false}`))
}

//go:wasmexport check_config_updates
func checkUpdates() uint32 {
	_ = takeShared()
	return setShared([]byte(`{"kind":"ok","data":[]}`))
}

//go:wasmexport set_file_path
func setPath() { _ = takeShared() }

//go:wasmexport set_override_config
func setOverride() { overrideError = emptySettings(takeShared()) }

//go:wasmexport format
func format(id uint32) uint32 { return formatText(id) }

//go:wasmexport format_range
func formatRange(id, start, end uint32) uint32 {
	_ = takeShared()
	overrideError = ""
	return fail("Caddy formatter does not support range formatting.")
}

//go:wasmexport get_formatted_text
func formattedText() uint32 {
	data := formatted
	formatted = nil
	return setShared(data)
}

//go:wasmexport get_error_text
func errorText() uint32 {
	message := lastError
	lastError = ""
	return setShared([]byte(message))
}
