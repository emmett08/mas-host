package generate

import (
	"encoding/binary"
	"encoding/json"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"syscall"
	"testing"
)

func write(t *testing.T, path, value string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(value), 0644); err != nil {
		t.Fatal(err)
	}
}

func TestGenerateBuildAndExecuteWithModuleReplacement(t *testing.T) {
	_, current, _, _ := runtime.Caller(0)
	sdk := filepath.Clean(filepath.Join(filepath.Dir(current), "../.."))
	project := t.TempDir()
	dependency := filepath.Join(project, "dependency")
	write(t, filepath.Join(dependency, "go.mod"), "module example.org/dependency\n\ngo 1.24.0\n")
	write(t, filepath.Join(dependency, "value.go"), "package dependency\nconst Value = 9007199254740993\n")
	write(t, filepath.Join(project, "go.mod"), "module example.org/application/v2\n\ngo 1.24.0\n\nrequire (\n github.com/emmett08/mas-host/sdk/go v0.0.0\n example.org/dependency v0.0.0\n)\nreplace example.org/dependency => ./dependency\n")
	write(t, filepath.Join(project, "internal", "worker", "application.go"), `package application
import (
 mas "github.com/emmett08/mas-host/sdk/go"
 "example.org/dependency"
)
type Input struct { Add int64 `+"`json:\"add\"`"+` }
type Output struct { Value int64 `+"`json:\"value\"`"+` }
func Calculate(_ *mas.Context,input Input)(Output,error) { return Output{Value:dependency.Value+input.Add},nil }
`)
	write(t, filepath.Join(project, "native-agents.json"), `{"version":"mas-native-build/1","agents":[{"name":"calculator","language":"go","source":"internal/worker","handler":"Calculate"}]}`)
	goBinary := filepath.Join(runtime.GOROOT(), "bin", "go")
	workers, err := Generate(Options{Agents: filepath.Join(project, "native-agents.json"), Project: project, Output: filepath.Join(project, ".mas", "build"), SDKDir: sdk, Go: goBinary})
	if err != nil {
		t.Fatal(err)
	}
	if len(workers) != 1 || workers[0].Handler != "Calculate" {
		t.Fatalf("wrong registry: %#v", workers)
	}
	if _, err := os.Stat(filepath.Join(project, "main.go")); !os.IsNotExist(err) {
		t.Fatal("generator wrote application source")
	}
	fds, err := syscall.Socketpair(syscall.AF_UNIX, syscall.SOCK_STREAM, 0)
	if err != nil {
		t.Fatal(err)
	}
	host := os.NewFile(uintptr(fds[0]), "host")
	defer host.Close()
	child := os.NewFile(uintptr(fds[1]), "worker")
	cmd := exec.Command(workers[0].Executable)
	cmd.Env = []string{"MAS_WORKER_CHANNEL_FD=3"}
	cmd.ExtraFiles = []*os.File{child}
	if err := cmd.Start(); err != nil {
		child.Close()
		t.Fatal(err)
	}
	child.Close()
	t.Cleanup(func() {
		if cmd.ProcessState == nil {
			cmd.Process.Kill()
			cmd.Wait()
		}
	})
	raw := []byte(`{"type":"start","version":"mas-native/1","actor":"calculator","handler":"Calculate","input":{"add":2},"invocation_id":"test","deadline_ms":0}`)
	var header [4]byte
	binary.BigEndian.PutUint32(header[:], uint32(len(raw)))
	if _, err := host.Write(append(header[:], raw...)); err != nil {
		t.Fatal(err)
	}
	if _, err := io.ReadFull(host, header[:]); err != nil {
		t.Fatal(err)
	}
	size := binary.BigEndian.Uint32(header[:])
	if size > 65536 {
		t.Fatal("oversized response")
	}
	response := make([]byte, size)
	if _, err := io.ReadFull(host, response); err != nil {
		t.Fatal(err)
	}
	if string(response) != `{"output":{"value":9007199254740995},"type":"complete"}` {
		t.Fatalf("wrong worker output %s", response)
	}
	if err := cmd.Wait(); err != nil {
		t.Fatal(err)
	}
	var registry []Worker
	content, err := os.ReadFile(filepath.Join(project, ".mas", "build", "go-workers.json"))
	if err != nil || json.Unmarshal(content, &registry) != nil || len(registry) != 1 {
		t.Fatal("registry not written")
	}
}

func TestRejectSourceEscape(t *testing.T) {
	root := t.TempDir()
	outside := t.TempDir()
	if err := os.Symlink(outside, filepath.Join(root, "escape")); err != nil {
		t.Fatal(err)
	}
	if _, err := containedSource(root, "escape"); err == nil || !strings.Contains(err.Error(), "escapes") {
		t.Fatalf("source escape accepted: %v", err)
	}
}

func TestRejectMissingExportedHandler(t *testing.T) {
	root := t.TempDir()
	write(t, filepath.Join(root, "app.go"), "package app\nfunc Existing() {}\n")
	if err := exportedFunction(packageInfo{Dir: root, ImportPath: "example/app", GoFiles: []string{"app.go"}}, "Missing"); err == nil {
		t.Fatal("accepted nonexistent handler")
	}
}
