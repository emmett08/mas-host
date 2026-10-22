// Package mas connects typed application handlers to the MAS host.
// The host owns routing, durable execution and effect admission. Application
// code outside Effect callbacks must be deterministic and free of effects.
package mas

import (
	"context"
	"encoding/binary"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"os"
	"strconv"
	"sync"
	"time"
)

const ContractVersion = "mas-native/1"

type Handler func(*Context, json.RawMessage) (json.RawMessage, error)

// Adapt is called by generated launchers, rather than by application authors.
func Adapt[I any, O any](fn func(*Context, I) (O, error)) Handler {
	return func(ctx *Context, raw json.RawMessage) (json.RawMessage, error) {
		input, err := Decode[I](raw)
		if err != nil {
			return nil, fmt.Errorf("decode handler input: %w", err)
		}
		output, err := fn(ctx, input)
		if err != nil {
			return nil, err
		}
		return object(output)
	}
}

type Context struct {
	context.Context
	Actor        string
	InvocationID string
	channel      io.ReadWriter
	mu           sync.Mutex
}

type RequestSpec struct {
	Key    string `json:"key"`
	Target string `json:"target"`
	Input  any    `json:"input"`
}

type EffectResult struct {
	Output   any
	Evidence any
}

type reply struct {
	Type         string            `json:"type"`
	OK           bool              `json:"ok"`
	Output       json.RawMessage   `json:"output"`
	Outputs      []json.RawMessage `json:"outputs"`
	Error        json.RawMessage   `json:"error"`
	State        string            `json:"state"`
	InvocationID string            `json:"invocation_id"`
	Completion   json.RawMessage   `json:"completion"`
}

func (c *Context) exchange(command any) (reply, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	var response reply
	if err := c.Err(); err != nil {
		return response, err
	}
	if err := writeFrame(c.channel, command); err != nil {
		return response, err
	}
	raw, err := readFrame(c.channel)
	if err != nil {
		return response, err
	}
	if err := json.Unmarshal(raw, &response); err != nil {
		return response, err
	}
	if response.Type == "response" && !response.OK {
		return response, fmt.Errorf("host rejected operation: %s", response.Error)
	}
	return response, nil
}

func (c *Context) Request(key, target string, input any) (json.RawMessage, error) {
	if key == "" || target == "" {
		return nil, fmt.Errorf("request requires a key and target")
	}
	value, err := object(input)
	if err != nil {
		return nil, err
	}
	response, err := c.exchange(map[string]any{"type": "request", "key": key, "target": target, "input": value})
	if err != nil {
		return nil, err
	}
	if response.Type != "response" || !response.OK {
		return nil, fmt.Errorf("unexpected request reply")
	}
	if err := ValidateObject(response.Output); err != nil {
		return nil, err
	}
	return response.Output, nil
}

func (c *Context) Parallel(requests []RequestSpec) ([]json.RawMessage, error) {
	if len(requests) == 0 {
		return nil, fmt.Errorf("parallel requires at least one request")
	}
	seen := make(map[string]bool)
	for _, request := range requests {
		if request.Key == "" || request.Target == "" || seen[request.Key] {
			return nil, fmt.Errorf("parallel requires distinct nonempty keys and targets")
		}
		seen[request.Key] = true
		if _, err := object(request.Input); err != nil {
			return nil, err
		}
	}
	response, err := c.exchange(map[string]any{"type": "parallel", "requests": requests})
	if err != nil {
		return nil, err
	}
	if response.Type != "response" || !response.OK || len(response.Outputs) != len(requests) {
		return nil, fmt.Errorf("unexpected parallel reply")
	}
	for _, output := range response.Outputs {
		if err := ValidateObject(output); err != nil {
			return nil, err
		}
	}
	return response.Outputs, nil
}

// Effect runs fn only after durable host admission. A cached result skips fn.
// Failure before acknowledged completion requires reconciliation, never retry.
// A governed completion returns the host's safe projection, also on replay.
// The invocation ID is stable and can serve as an external idempotency key.
func (c *Context) Effect(key string, descriptor any, fn func(string) (EffectResult, error)) (json.RawMessage, error) {
	if key == "" || fn == nil {
		return nil, fmt.Errorf("effect requires key and callback")
	}
	value, err := object(descriptor)
	if err != nil {
		return nil, err
	}
	response, err := c.exchange(map[string]any{"type": "effect_begin", "key": key, "descriptor": value})
	if err != nil {
		return nil, err
	}
	if response.Type != "effect" {
		return nil, fmt.Errorf("unexpected effect admission reply")
	}
	if response.State == "cached" {
		if err := ValidateObject(response.Output); err != nil {
			return nil, err
		}
		return response.Output, nil
	}
	if response.State != "execute" || response.InvocationID == "" {
		return nil, fmt.Errorf("invalid effect admission")
	}
	result, err := fn(response.InvocationID)
	if err != nil {
		return nil, fmt.Errorf("admitted effect requires reconciliation: %w", err)
	}
	output, err := object(result.Output)
	if err != nil {
		return nil, err
	}
	evidence, err := object(result.Evidence)
	if err != nil {
		return nil, err
	}
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(evidence, &fields); err != nil || len(fields) == 0 {
		return nil, fmt.Errorf("effect evidence must be a nonempty object")
	}
	complete, err := c.exchange(map[string]any{"type": "effect_complete", "key": key, "output": output, "evidence": evidence})
	if err != nil {
		return nil, err
	}
	if complete.Type != "response" || !complete.OK {
		return nil, fmt.Errorf("unexpected effect completion reply")
	}
	return completedOutput(complete.Completion, response.InvocationID, output)
}

func completedOutput(raw json.RawMessage, effectID string, original json.RawMessage) (json.RawMessage, error) {
	if len(raw) == 0 {
		return original, nil
	}
	invalid := fmt.Errorf("invalid governed effect completion")
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(raw, &fields); err != nil || len(fields) != 6 {
		return nil, invalid
	}
	text := func(key string) string {
		var value string
		if err := json.Unmarshal(fields[key], &value); err != nil {
			return ""
		}
		return value
	}
	execution, disposition := text("external_execution"), text("payload_disposition")
	if text("version") != "mas-governed-effect-completion/1" || text("effect_id") != effectID || text("host_settlement") != "completed" ||
		(execution != "completed" && execution != "not-sent" && execution != "refused" && execution != "unknown") ||
		(disposition != "available" && disposition != "withheld") || ValidateObject(fields["output"]) != nil {
		return nil, invalid
	}
	if disposition == "withheld" {
		var marker map[string]string
		if err := json.Unmarshal(fields["output"], &marker); err != nil || len(marker) != 3 || marker["version"] != "mas-governed-unavailable/1" || marker["external_execution"] != execution || marker["payload_disposition"] != "withheld" {
			return nil, invalid
		}
	} else {
		var outputFields map[string]json.RawMessage
		_ = json.Unmarshal(fields["output"], &outputFields)
		var version string
		_ = json.Unmarshal(outputFields["version"], &version)
		if version == "mas-governed-unavailable/1" {
			return nil, invalid
		}
	}
	return fields["output"], nil
}

func readFrame(reader io.Reader) (json.RawMessage, error) {
	var header [4]byte
	if _, err := io.ReadFull(reader, header[:]); err != nil {
		return nil, err
	}
	length := binary.BigEndian.Uint32(header[:])
	if length == 0 || length > MaxFrameBytes {
		return nil, fmt.Errorf("invalid native frame length %d", length)
	}
	raw := make([]byte, int(length))
	if _, err := io.ReadFull(reader, raw); err != nil {
		return nil, err
	}
	if err := ValidateObject(raw); err != nil {
		return nil, err
	}
	return raw, nil
}

func writeFrame(writer io.Writer, value any) error {
	raw, err := object(value)
	if err != nil {
		return err
	}
	frame := make([]byte, 4+len(raw))
	binary.BigEndian.PutUint32(frame[:4], uint32(len(raw)))
	copy(frame[4:], raw)
	for len(frame) > 0 {
		n, err := writer.Write(frame)
		if err != nil {
			return err
		}
		if n == 0 {
			return io.ErrShortWrite
		}
		frame = frame[n:]
	}
	return nil
}

// Serve is the generated worker entry point. Standard output is never framing.
func Serve(handlers map[string]Handler) error {
	fd, err := strconv.Atoi(os.Getenv("MAS_WORKER_CHANNEL_FD"))
	if err != nil || fd < 3 {
		return fmt.Errorf("MAS_WORKER_CHANNEL_FD must identify a private socket")
	}
	file := os.NewFile(uintptr(fd), "mas-worker-channel")
	if file == nil {
		return fmt.Errorf("invalid worker channel descriptor")
	}
	connection, err := net.FileConn(file)
	file.Close()
	if err != nil {
		return err
	}
	defer connection.Close()
	return serveChannel(connection, handlers)
}

func serveChannel(channel net.Conn, handlers map[string]Handler) error {
	raw, err := readFrame(channel)
	if err != nil {
		return err
	}
	var start struct {
		Type         string          `json:"type"`
		Version      string          `json:"version"`
		Actor        string          `json:"actor"`
		Handler      string          `json:"handler"`
		Input        json.RawMessage `json:"input"`
		InvocationID string          `json:"invocation_id"`
		DeadlineMS   int64           `json:"deadline_ms"`
	}
	if err := json.Unmarshal(raw, &start); err != nil {
		return err
	}
	if start.Type != "start" || start.Version != ContractVersion || start.Actor == "" || start.InvocationID == "" {
		return fmt.Errorf("invalid native start frame")
	}
	if err := ValidateObject(start.Input); err != nil {
		return err
	}
	handler, ok := handlers[start.Handler]
	if !ok {
		return writeFrame(channel, map[string]any{"type": "error", "message": "unknown handler: " + start.Handler})
	}
	parent := context.Background()
	if start.DeadlineMS > 0 {
		deadline := time.UnixMilli(start.DeadlineMS)
		var cancel context.CancelFunc
		parent, cancel = context.WithDeadline(parent, deadline)
		defer cancel()
		if err := channel.SetDeadline(deadline); err != nil {
			return err
		}
	}
	ctx := &Context{Context: parent, Actor: start.Actor, InvocationID: start.InvocationID, channel: channel}
	output, callErr := invoke(ctx, handler, start.Input)
	if callErr != nil {
		return writeFrame(channel, map[string]any{"type": "error", "message": callErr.Error()})
	}
	if err := ValidateObject(output); err != nil {
		return writeFrame(channel, map[string]any{"type": "error", "message": err.Error()})
	}
	return writeFrame(channel, map[string]any{"type": "complete", "output": output})
}

func invoke(ctx *Context, handler Handler, input json.RawMessage) (output json.RawMessage, err error) {
	defer func() {
		if value := recover(); value != nil {
			err = fmt.Errorf("application handler panicked: %v", value)
		}
	}()
	return handler(ctx, input)
}
