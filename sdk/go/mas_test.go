package mas

import (
	"context"
	"encoding/binary"
	"encoding/json"
	"fmt"
	"net"
	"os"
	"strings"
	"syscall"
	"testing"
)

func TestStrictJSON(t *testing.T) {
	for _, raw := range []string{
		`{"n":9007199254740993}`, `{"n":18446744073709551615}`, `{"s":"\ud83d\ude00"}`, `{"s":"\\ud800"}`, `{"s":"λ😀"}`,
	} {
		if err := ValidateObject([]byte(raw)); err != nil {
			t.Fatalf("%s: %v", raw, err)
		}
	}
	for _, raw := range []string{
		`{"a":1,"a":2}`, `{"a":1,"\u0061":2}`, `{"n":1.0}`, `{"n":1e2}`, `{"s":"\ud800"}`, `{"s":"\udc00"}`, `{"s":"\ud800\u0041"}`, `{"s":"` + string([]byte{0xff}) + `"}`, `[]`, `{} {}`, `{"nested":` + strings.Repeat("[", 33) + `0` + strings.Repeat("]", 33) + `}`,
	} {
		if err := ValidateObject([]byte(raw)); err == nil {
			t.Fatalf("accepted %q", raw)
		}
	}
	value, err := Decode[map[string]any](json.RawMessage(`{"n":9007199254740993}`))
	if err != nil {
		t.Fatal(err)
	}
	if value["n"] != json.Number("9007199254740993") {
		t.Fatalf("integer lost precision: %#v", value)
	}
	if _, err := Decode[struct {
		N float64 `json:"n"`
	}](json.RawMessage(`{"n":9007199254740993}`)); err == nil {
		t.Fatal("decoded integer into rounding-prone float64")
	}
	if _, err := object(map[string]any{"n": float64(1)}); err == nil {
		t.Fatal("accepted implicit integral float coercion")
	}
	if _, err := object(map[string]any{"s": string([]byte{0xff})}); err == nil {
		t.Fatal("accepted invalid UTF-8 replacement")
	}
	if err := ValidateObject([]byte(`{"n":` + strings.Repeat("9", 1025) + `}`)); err == nil {
		t.Fatal("accepted integer longer than protocol limit")
	}
	if err := ValidateObject([]byte(`{"n":` + strings.Repeat("[", 31) + `0` + strings.Repeat("]", 31) + `}`)); err != nil {
		t.Fatalf("rejected boundary depth: %v", err)
	}
}

func unixPair(t *testing.T) (net.Conn, net.Conn) {
	t.Helper()
	descriptors, err := syscall.Socketpair(syscall.AF_UNIX, syscall.SOCK_STREAM, 0)
	if err != nil {
		t.Fatal(err)
	}
	first, second := os.NewFile(uintptr(descriptors[0]), "worker"), os.NewFile(uintptr(descriptors[1]), "host")
	client, err := net.FileConn(first)
	first.Close()
	if err != nil {
		second.Close()
		t.Fatal(err)
	}
	server, err := net.FileConn(second)
	second.Close()
	if err != nil {
		client.Close()
		t.Fatal(err)
	}
	t.Cleanup(func() { client.Close(); server.Close() })
	return client, server
}

func hostRead(t *testing.T, channel net.Conn, kind string) map[string]json.RawMessage {
	t.Helper()
	raw, err := readFrame(channel)
	if err != nil {
		t.Fatal(err)
	}
	var value map[string]json.RawMessage
	if err := json.Unmarshal(raw, &value); err != nil {
		t.Fatal(err)
	}
	if string(value["type"]) != fmt.Sprintf("%q", kind) {
		t.Fatalf("wanted %s got %s", kind, raw)
	}
	return value
}

func hostSend(t *testing.T, channel net.Conn, value any) {
	t.Helper()
	if err := writeFrame(channel, value); err != nil {
		t.Fatal(err)
	}
}

func TestUnixProtocolDelegationAndEffectReplay(t *testing.T) {
	worker, host := unixPair(t)
	called := 0
	result := make(chan error, 1)
	go func() {
		result <- serveChannel(worker, map[string]Handler{"Run": Adapt(func(ctx *Context, input map[string]any) (map[string]any, error) {
			if ctx.Actor != "planner" || ctx.InvocationID != "inv-1" {
				return nil, fmt.Errorf("wrong context")
			}
			output, err := ctx.Request("child", "checker", input)
			if err != nil {
				return nil, err
			}
			outputs, err := ctx.Parallel([]RequestSpec{{Key: "one", Target: "a", Input: input}, {Key: "two", Target: "b", Input: input}})
			if err != nil {
				return nil, err
			}
			if len(outputs) != 2 {
				return nil, fmt.Errorf("missing parallel result")
			}
			callback := func(id string) (EffectResult, error) {
				called++
				if id != "effect-1" {
					return EffectResult{}, fmt.Errorf("wrong admission")
				}
				return EffectResult{Output: output, Evidence: map[string]any{"receipt": "verified"}}, nil
			}
			_, err = ctx.Effect("write", map[string]any{"operation": "write"}, callback)
			if err != nil {
				return nil, err
			}
			cached, err := ctx.Effect("write", map[string]any{"operation": "write"}, callback)
			if err != nil {
				return nil, err
			}
			return Decode[map[string]any](cached)
		})})
	}()
	input := json.RawMessage(`{"n":9007199254740993}`)
	hostSend(t, host, map[string]any{"type": "start", "version": ContractVersion, "actor": "planner", "handler": "Run", "input": input, "invocation_id": "inv-1", "deadline_ms": 0})
	request := hostRead(t, host, "request")
	if string(request["input"]) != string(input) {
		t.Fatalf("request integer changed: %s", request["input"])
	}
	hostSend(t, host, map[string]any{"type": "response", "ok": true, "output": input})
	hostRead(t, host, "parallel")
	hostSend(t, host, map[string]any{"type": "response", "ok": true, "outputs": []json.RawMessage{input, input}})
	hostRead(t, host, "effect_begin")
	hostSend(t, host, map[string]any{"type": "effect", "state": "execute", "invocation_id": "effect-1"})
	hostRead(t, host, "effect_complete")
	hostSend(t, host, map[string]any{"type": "response", "ok": true})
	hostRead(t, host, "effect_begin")
	hostSend(t, host, map[string]any{"type": "effect", "state": "cached", "output": input})
	final := hostRead(t, host, "complete")
	if string(final["output"]) != string(input) {
		t.Fatalf("output integer changed: %s", final["output"])
	}
	if err := <-result; err != nil {
		t.Fatal(err)
	}
	if called != 1 {
		t.Fatalf("effect callback ran %d times", called)
	}
}

func TestRejectedAdmissionDoesNotRunCallback(t *testing.T) {
	worker, host := unixPair(t)
	result := make(chan error, 1)
	go func() {
		ctx := &Context{Context: context.Background(), channel: worker}
		_, err := ctx.Effect("write", map[string]any{}, func(string) (EffectResult, error) { return EffectResult{}, fmt.Errorf("CALLBACK EXECUTED") })
		result <- err
	}()
	hostRead(t, host, "effect_begin")
	hostSend(t, host, map[string]any{"type": "response", "ok": false, "error": "budget_exhausted"})
	if err := <-result; err == nil || !strings.Contains(err.Error(), "budget_exhausted") {
		t.Fatalf("unexpected error %v", err)
	}
}

func TestFrameBoundBeforeAllocation(t *testing.T) {
	for _, size := range []uint32{0, MaxFrameBytes + 1, ^uint32(0)} {
		var header [4]byte
		binary.BigEndian.PutUint32(header[:], size)
		if _, err := readFrame(strings.NewReader(string(header[:]))); err == nil {
			t.Fatalf("accepted length %d", size)
		}
	}
}
