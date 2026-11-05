package main

import (
	"bytes"
	"testing"
)

func TestConfigurationAndRecovery(t *testing.T) {
	setShared([]byte(`{"plugin":{},"global":{"useTabs":false,"indentWidth":8}}`))
	registerConfig(1)
	setShared([]byte("localhost { respond ok }"))
	if formatText(1) != 1 || string(formatted) != "localhost {\n\trespond ok\n}\n" {
		t.Fatal(string(formatted), lastError)
	}
	setShared(formatted)
	if formatText(1) != 0 {
		t.Fatal("not idempotent")
	}
	setShared([]byte(`{"plugin":{"indentWidth":8}}`))
	registerConfig(2)
	setShared([]byte("localhost {}"))
	if formatText(2) != 2 {
		t.Fatal("options accepted")
	}
	delete(configs, 2)
	setShared([]byte{0xff})
	if formatText(1) != 2 {
		t.Fatal("invalid UTF-8 accepted")
	}
	setShared([]byte("localhost {}"))
	if formatText(1) != 1 {
		t.Fatal(lastError)
	}
}

func TestBounds(t *testing.T) {
	setShared(bytes.Repeat([]byte(" "), limit+1))
	if formatText(0) != 2 {
		t.Fatal("oversized input accepted")
	}
	setShared(bytes.Repeat([]byte(" "), limit))
	if formatText(0) != 1 || string(formatted) != "\n" {
		t.Fatal("exact bound rejected")
	}
	setShared(bytes.Repeat([]byte("x"), limit))
	if formatText(0) != 2 {
		t.Fatal("oversized output accepted")
	}
}
