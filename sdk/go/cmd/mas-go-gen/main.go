// mas-go-gen builds native workers without editing application source.
package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"runtime"

	"github.com/emmett08/mas-host/sdk/go/internal/generate"
)

func main() {
	_, current, _, _ := runtime.Caller(0)
	var options generate.Options
	flag.StringVar(&options.Project, "project", ".", "application project directory")
	flag.StringVar(&options.Output, "output", "", "generated build directory")
	flag.StringVar(&options.Agents, "agents", "", "generated native build manifest (required)")
	flag.StringVar(&options.SDKDir, "sdk-dir", filepath.Clean(filepath.Join(filepath.Dir(current), "../..")), "installed MAS Go SDK directory")
	flag.StringVar(&options.Go, "go", "go", "Go compiler executable")
	flag.Parse()
	if options.Output == "" {
		options.Output = filepath.Join(options.Project, ".mas", "build")
	}
	workers, err := generate.Generate(options)
	if err != nil {
		fmt.Fprintln(os.Stderr, "mas-go-gen:", err)
		os.Exit(1)
	}
	if err := json.NewEncoder(os.Stdout).Encode(workers); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
