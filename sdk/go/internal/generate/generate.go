// Package generate creates disposable workers from ordinary exported functions.
package generate

import (
	"bytes"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"go/ast"
	"go/format"
	"go/parser"
	"go/token"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"

	mas "github.com/emmett08/mas-host/sdk/go"
)

type Options struct {
	Project string
	Output  string
	Agents  string
	SDKDir  string
	Go      string
}

type Agent struct {
	Name     string `json:"name"`
	Language string `json:"language"`
	Source   string `json:"source"`
	Handler  string `json:"handler"`
}

type Worker struct {
	Name       string `json:"name"`
	Executable string `json:"executable"`
	Handler    string `json:"handler"`
}

type packageInfo struct {
	Dir        string
	ImportPath string
	Name       string
	GoFiles    []string
	Module     *struct {
		Path string
		Dir  string
	}
	Error *struct{ Err string }
}

// Generate never edits application source. The compiler checks handler types
// through mas.Adapt's inferred type parameters in each generated main package.
func Generate(options Options) ([]Worker, error) {
	if options.Go == "" {
		options.Go = "go"
	}
	project, err := filepath.Abs(options.Project)
	if err != nil {
		return nil, err
	}
	project, err = filepath.EvalSymlinks(project)
	if err != nil {
		return nil, err
	}
	output, err := filepath.Abs(options.Output)
	if err != nil {
		return nil, err
	}
	sdk, err := filepath.Abs(options.SDKDir)
	if err != nil || options.SDKDir == "" {
		return nil, fmt.Errorf("--sdk-dir must identify the MAS Go SDK")
	}
	if _, err := os.Stat(filepath.Join(sdk, "go.mod")); err != nil {
		return nil, fmt.Errorf("SDK directory: %w", err)
	}
	agentsPath := options.Agents
	if agentsPath == "" {
		return nil, fmt.Errorf("--agents must identify the generated native build manifest")
	}
	raw, err := os.ReadFile(agentsPath)
	if err != nil {
		return nil, err
	}
	if err := mas.ValidateObject(raw); err != nil {
		return nil, fmt.Errorf("native build manifest: %w", err)
	}
	var configuration struct {
		Version string  `json:"version"`
		Agents  []Agent `json:"agents"`
	}
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&configuration); err != nil {
		return nil, err
	}
	if configuration.Version != "mas-native-build/1" {
		return nil, fmt.Errorf("unsupported native build manifest version %q", configuration.Version)
	}
	if err := os.MkdirAll(output, 0755); err != nil {
		return nil, err
	}
	packages := make(map[string]packageInfo)
	groups := make(map[string][]Agent)
	seen := make(map[string]bool)
	for _, agent := range configuration.Agents {
		if agent.Name == "" || agent.Name == "client" || seen[agent.Name] {
			return nil, fmt.Errorf("invalid or duplicate agent name %q", agent.Name)
		}
		seen[agent.Name] = true
		if agent.Language != "go" {
			continue
		}
		if !token.IsIdentifier(agent.Handler) || !ast.IsExported(agent.Handler) {
			return nil, fmt.Errorf("agent %s: handler must be an exported Go function", agent.Name)
		}
		source, err := containedSource(project, agent.Source)
		if err != nil {
			return nil, fmt.Errorf("agent %s: %w", agent.Name, err)
		}
		info, exists := packages[source]
		if !exists {
			workspace := exec.Command(options.Go, "env", "GOWORK")
			workspace.Dir = source
			active, err := workspace.Output()
			if err != nil {
				return nil, commandError("go env GOWORK", err)
			}
			if value := strings.TrimSpace(string(active)); value != "" && value != "off" {
				return nil, fmt.Errorf("active Go workspace %s is not supported; use a standalone application module or explicitly GOWORK=off", value)
			}
			cmd := exec.Command(options.Go, "list", "-e", "-json", ".")
			cmd.Dir = source
			cmd.Env = withoutWorkspace()
			result, err := cmd.Output()
			if err != nil {
				return nil, commandError("go list", err)
			}
			if err := json.Unmarshal(result, &info); err != nil {
				return nil, err
			}
			if info.Module == nil || info.Module.Path == "" {
				return nil, fmt.Errorf("%s must belong to a Go module", source)
			}
			if info.Name == "main" || info.Name == "" {
				return nil, fmt.Errorf("%s: handlers must be in an importable Go package", source)
			}
			packages[source] = info
		}
		if err := exportedFunction(info, agent.Handler); err != nil {
			return nil, err
		}
		groups[source] = append(groups[source], agent)
	}
	keys := make([]string, 0, len(groups))
	for key := range groups {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	workers := []Worker{}
	for _, source := range keys {
		info := packages[source]
		digest := sha256.Sum256([]byte(info.ImportPath))
		identifier := fmt.Sprintf("%x", digest[:8])
		buildDir := filepath.Join(output, "go", identifier)
		if err := os.MkdirAll(buildDir, 0755); err != nil {
			return nil, err
		}
		mod, err := generatedModule(options.Go, info, identifier, sdk)
		if err != nil {
			return nil, err
		}
		if err := os.WriteFile(filepath.Join(buildDir, "go.mod"), []byte(mod), 0644); err != nil {
			return nil, err
		}
		if sums, err := os.ReadFile(filepath.Join(info.Module.Dir, "go.sum")); err == nil {
			if err := os.WriteFile(filepath.Join(buildDir, "go.sum"), sums, 0644); err != nil {
				return nil, err
			}
		} else if !os.IsNotExist(err) {
			return nil, err
		}
		var code strings.Builder
		fmt.Fprintf(&code, "// Code generated by mas-go-gen. DO NOT EDIT.\npackage main\nimport (\n \"fmt\"\n \"os\"\n mas \"github.com/emmett08/mas-host/sdk/go\"\n app %q\n)\nfunc main() {\n if err := mas.Serve(map[string]mas.Handler{\n", info.ImportPath)
		handlers := make(map[string]bool)
		for _, agent := range groups[source] {
			if !handlers[agent.Handler] {
				fmt.Fprintf(&code, "%q: mas.Adapt(app.%s),\n", agent.Handler, agent.Handler)
				handlers[agent.Handler] = true
			}
		}
		code.WriteString("}); err != nil { fmt.Fprintln(os.Stderr, err); os.Exit(1) }\n}\n")
		formatted, err := format.Source([]byte(code.String()))
		if err != nil {
			return nil, err
		}
		if err := os.WriteFile(filepath.Join(buildDir, "main.go"), formatted, 0644); err != nil {
			return nil, err
		}
		executable := filepath.Join(output, "go-worker-"+identifier)
		cmd := exec.Command(options.Go, "build", "-mod=readonly", "-trimpath", "-o", executable, ".")
		cmd.Dir = buildDir
		cmd.Env = withoutWorkspace()
		if result, err := cmd.CombinedOutput(); err != nil {
			return nil, fmt.Errorf("compile handlers in %s: %w\n%s", info.ImportPath, err, result)
		}
		for _, agent := range groups[source] {
			workers = append(workers, Worker{Name: agent.Name, Executable: executable, Handler: agent.Handler})
		}
	}
	sort.Slice(workers, func(i, j int) bool { return workers[i].Name < workers[j].Name })
	registry, err := json.MarshalIndent(workers, "", "  ")
	if err != nil {
		return nil, err
	}
	registry = append(registry, '\n')
	temporary := filepath.Join(output, "go-workers.json.tmp")
	if err := os.WriteFile(temporary, registry, 0644); err != nil {
		return nil, err
	}
	if err := os.Rename(temporary, filepath.Join(output, "go-workers.json")); err != nil {
		return nil, err
	}
	return workers, nil
}

func generatedModule(goBinary string, info packageInfo, identifier, sdk string) (string, error) {
	cmd := exec.Command(goBinary, "mod", "edit", "-json")
	cmd.Dir = info.Module.Dir
	cmd.Env = withoutWorkspace()
	raw, err := cmd.Output()
	if err != nil {
		return "", commandError("go mod edit -json", err)
	}
	type moduleVersion struct {
		Path    string
		Version string
	}
	var source struct {
		Go      string
		Require []moduleVersion
		Replace []struct {
			Old moduleVersion
			New moduleVersion
		}
		Exclude []moduleVersion
	}
	if err := json.Unmarshal(raw, &source); err != nil {
		return "", err
	}
	goVersion := "1.24.0"
	var minor int
	if _, err := fmt.Sscanf(source.Go, "1.%d", &minor); err == nil && minor >= 24 {
		goVersion = source.Go
	}
	var mod strings.Builder
	sdkVersion := "v0.0.0"
	for _, requirement := range source.Require {
		if requirement.Path == "github.com/emmett08/mas-host/sdk/go" {
			sdkVersion = requirement.Version
		}
	}
	fmt.Fprintf(&mod, "module %s/masgenerated/%s\n\ngo %s\n\nrequire (\n %s %s\n github.com/emmett08/mas-host/sdk/go %s\n", info.Module.Path, identifier, goVersion, info.Module.Path, moduleVersionForPath(info.Module.Path), sdkVersion)
	for _, requirement := range source.Require {
		if requirement.Path != info.Module.Path && requirement.Path != "github.com/emmett08/mas-host/sdk/go" {
			fmt.Fprintf(&mod, " %s %s\n", requirement.Path, requirement.Version)
		}
	}
	mod.WriteString(")\n")
	fmt.Fprintf(&mod, "replace %s => %s\nreplace github.com/emmett08/mas-host/sdk/go => %s\n", info.Module.Path, strconv.Quote(info.Module.Dir), strconv.Quote(sdk))
	for _, replacement := range source.Replace {
		if replacement.Old.Path == info.Module.Path || replacement.Old.Path == "github.com/emmett08/mas-host/sdk/go" {
			continue
		}
		newPath := replacement.New.Path
		if replacement.New.Version == "" && !filepath.IsAbs(newPath) {
			newPath = filepath.Join(info.Module.Dir, newPath)
		}
		fmt.Fprintf(&mod, "replace %s %s => %s %s\n", replacement.Old.Path, replacement.Old.Version, strconv.Quote(newPath), replacement.New.Version)
	}
	for _, excluded := range source.Exclude {
		fmt.Fprintf(&mod, "exclude %s %s\n", excluded.Path, excluded.Version)
	}
	return mod.String(), nil
}

func containedSource(root, source string) (string, error) {
	if source == "" || filepath.IsAbs(source) {
		return "", fmt.Errorf("source must be a project-relative directory")
	}
	path, err := filepath.EvalSymlinks(filepath.Join(root, source))
	if err != nil {
		return "", err
	}
	relative, err := filepath.Rel(root, path)
	if err != nil || relative == ".." || strings.HasPrefix(relative, ".."+string(filepath.Separator)) {
		return "", fmt.Errorf("source escapes project directory")
	}
	info, err := os.Stat(path)
	if err != nil || !info.IsDir() {
		return "", fmt.Errorf("source is not a directory")
	}
	return path, nil
}

func exportedFunction(info packageInfo, name string) error {
	for _, filename := range info.GoFiles {
		file, err := parser.ParseFile(token.NewFileSet(), filepath.Join(info.Dir, filename), nil, 0)
		if err != nil {
			return err
		}
		for _, declaration := range file.Decls {
			if fn, ok := declaration.(*ast.FuncDecl); ok && fn.Recv == nil && fn.Name.Name == name {
				return nil
			}
		}
	}
	return fmt.Errorf("package %s has no exported function %s", info.ImportPath, name)
}

var majorSuffix = regexp.MustCompile(`/v([2-9][0-9]*)$`)

func moduleVersionForPath(path string) string {
	if match := majorSuffix.FindStringSubmatch(path); match != nil {
		return "v" + match[1] + ".0.0"
	}
	return "v0.0.0"
}

func withoutWorkspace() []string {
	env := []string{}
	for _, entry := range os.Environ() {
		if !strings.HasPrefix(entry, "GOWORK=") {
			env = append(env, entry)
		}
	}
	return append(env, "GOWORK=off")
}

func commandError(name string, err error) error {
	if exit, ok := err.(*exec.ExitError); ok {
		return fmt.Errorf("%s: %w: %s", name, err, exit.Stderr)
	}
	return fmt.Errorf("%s: %w", name, err)
}
