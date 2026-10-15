// Package release qualifies a proposed release using independent assessments.
// This package contains business decisions; the host generates its executable.
package release

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strconv"

	mas "github.com/emmett08/mas-host/sdk/go"
)

type Proposal struct {
	Release                 string `json:"release"`
	Replicas                int64  `json:"replicas"`
	Demand                  int64  `json:"demand"`
	PerReplica              int64  `json:"per_replica"`
	CriticalVulnerabilities int64  `json:"critical_vulnerabilities"`
	MaxReplicas             int64  `json:"max_replicas"`
	ReceiptPath             string `json:"receipt_path,omitempty"`
	CrashAfterReceipt       bool   `json:"crash_after_receipt,omitempty"`
}

type Assessment struct {
	Ready  bool   `json:"ready"`
	Reason string `json:"reason"`
}

type Decision struct {
	Release  string     `json:"release"`
	Ready    bool       `json:"ready"`
	Replicas int64      `json:"replicas"`
	Attempts int64      `json:"attempts"`
	Capacity Assessment `json:"capacity"`
	Security Assessment `json:"security"`
}

type CheckInput struct {
	Proposal Proposal `json:"proposal"`
	Decision Decision `json:"decision"`
}

// Qualify performs independent checks in parallel and allows at most two
// capacity revisions. Security failure stops immediately. Only the authorised
// topology targets can receive the requests below.
func Qualify(ctx *mas.Context, proposal Proposal) (Decision, error) {
	if err := validate(proposal); err != nil {
		return Decision{}, err
	}
	var decision Decision
	for attempt := int64(1); attempt <= 3; attempt++ {
		prefix := "attempt_" + strconv.FormatInt(attempt, 10)
		results, err := ctx.Parallel([]mas.RequestSpec{
			{Key: prefix + "_capacity", Target: "capacity", Input: proposal},
			{Key: prefix + "_security", Target: "security", Input: proposal},
		})
		if err != nil {
			return Decision{}, err
		}
		capacity, err := mas.Decode[Assessment](results[0])
		if err != nil {
			return Decision{}, err
		}
		security, err := mas.Decode[Assessment](results[1])
		if err != nil {
			return Decision{}, err
		}
		decision = Decision{Release: proposal.Release, Ready: capacity.Ready && security.Ready, Replicas: proposal.Replicas, Attempts: attempt, Capacity: capacity, Security: security}
		if decision.Ready || !security.Ready || proposal.Replicas >= proposal.MaxReplicas || attempt == 3 {
			break
		}
		proposal.Replicas++
	}
	checked, err := ctx.Request("independent_acceptance", "checker", CheckInput{Proposal: proposal, Decision: decision})
	if err != nil {
		return Decision{}, err
	}
	decision, err = mas.Decode[Decision](checked)
	if err != nil || proposal.ReceiptPath == "" {
		return decision, err
	}
	// This optional local audit publication is the explicit effect boundary.
	// The crash switch exists solely to demonstrate uncertain-effect recovery.
	recorded, err := ctx.Effect("publish_receipt", map[string]any{"operation": "publish_release_receipt", "path": proposal.ReceiptPath, "decision": decision}, func(invocationID string) (mas.EffectResult, error) {
		if err := writeReceipt(proposal.ReceiptPath, invocationID, decision); err != nil {
			return mas.EffectResult{}, err
		}
		if proposal.CrashAfterReceipt {
			os.Exit(70)
		}
		return mas.EffectResult{Output: decision, Evidence: map[string]any{"receipt_path": proposal.ReceiptPath, "invocation_id": invocationID}}, nil
	})
	if err != nil {
		return Decision{}, err
	}
	return mas.Decode[Decision](recorded)
}

func AssessCapacity(_ *mas.Context, proposal Proposal) (Assessment, error) {
	if err := validate(proposal); err != nil {
		return Assessment{}, err
	}
	// Division avoids overflow in replicas * per_replica.
	required := proposal.Demand / proposal.PerReplica
	if proposal.Demand%proposal.PerReplica != 0 {
		required++
	}
	if proposal.Replicas < required {
		return Assessment{Ready: false, Reason: "insufficient capacity"}, nil
	}
	return Assessment{Ready: true, Reason: "demand fits provisioned capacity"}, nil
}

func AssessSecurity(_ *mas.Context, proposal Proposal) (Assessment, error) {
	if err := validate(proposal); err != nil {
		return Assessment{}, err
	}
	if proposal.CriticalVulnerabilities != 0 {
		return Assessment{Ready: false, Reason: "critical vulnerabilities remain"}, nil
	}
	return Assessment{Ready: true, Reason: "no declared critical vulnerabilities"}, nil
}

// Check independently evaluates the decision rather than trusting the two
// reports. It does not call either assessment implementation.
func Check(_ *mas.Context, input CheckInput) (Decision, error) {
	p := input.Proposal
	d := input.Decision
	if err := validate(p); err != nil {
		return Decision{}, err
	}
	// Demand / replicas is safe even at maximum signed-integer input.
	capacityReady := p.PerReplica >= p.Demand/p.Replicas
	if p.PerReplica == p.Demand/p.Replicas && p.Demand%p.Replicas != 0 {
		capacityReady = false
	}
	securityReady := p.CriticalVulnerabilities == 0
	if d.Release != p.Release || d.Replicas != p.Replicas || d.Attempts < 1 || d.Attempts > 3 || d.Capacity.Ready != capacityReady || d.Security.Ready != securityReady || d.Ready != (capacityReady && securityReady) {
		return Decision{}, fmt.Errorf("qualification reports disagree with independently evaluated proposal")
	}
	return d, nil
}

func validate(proposal Proposal) error {
	if proposal.Release == "" || proposal.Replicas < 1 || proposal.PerReplica < 1 || proposal.Demand < 0 || proposal.CriticalVulnerabilities < 0 || proposal.MaxReplicas < proposal.Replicas || proposal.MaxReplicas > 1000000 {
		return fmt.Errorf("release, positive capacity, nonnegative demand/vulnerabilities and replicas <= max_replicas <= 1000000 are required")
	}
	if proposal.ReceiptPath != "" && !filepath.IsAbs(proposal.ReceiptPath) {
		return fmt.Errorf("receipt_path must be absolute")
	}
	if proposal.CrashAfterReceipt && proposal.ReceiptPath == "" {
		return fmt.Errorf("crash_after_receipt requires receipt_path")
	}
	return nil
}

func writeReceipt(path, invocationID string, decision Decision) error {
	raw, err := json.Marshal(map[string]any{"invocation_id": invocationID, "output": decision})
	if err != nil {
		return err
	}
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
	if err != nil {
		return err
	}
	if _, err := file.Write(append(raw, '\n')); err != nil {
		file.Close()
		return err
	}
	if err := file.Sync(); err != nil {
		file.Close()
		return err
	}
	if err := file.Close(); err != nil {
		return err
	}
	directory, err := os.Open(filepath.Dir(path))
	if err != nil {
		return err
	}
	defer directory.Close()
	return directory.Sync()
}
