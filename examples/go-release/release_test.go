package release

import (
	"math"
	"testing"
)

func TestAssessmentsAndIndependentCheck(t *testing.T) {
	for _, test := range []struct {
		name     string
		proposal Proposal
		ready    bool
	}{
		{"ready", Proposal{Release: "r1", Replicas: 2, Demand: 180, PerReplica: 100, MaxReplicas: 4}, true},
		{"capacity", Proposal{Release: "r1", Replicas: 1, Demand: 180, PerReplica: 100, MaxReplicas: 4}, false},
		{"security", Proposal{Release: "r1", Replicas: 2, Demand: 180, PerReplica: 100, CriticalVulnerabilities: 1, MaxReplicas: 4}, false},
		{"large_integer", Proposal{Release: "r1", Replicas: 3, Demand: math.MaxInt64, PerReplica: math.MaxInt64 / 2, MaxReplicas: 4}, true},
	} {
		t.Run(test.name, func(t *testing.T) {
			capacity, err := AssessCapacity(nil, test.proposal)
			if err != nil {
				t.Fatal(err)
			}
			security, err := AssessSecurity(nil, test.proposal)
			if err != nil {
				t.Fatal(err)
			}
			decision := Decision{Release: "r1", Ready: capacity.Ready && security.Ready, Replicas: test.proposal.Replicas, Attempts: 1, Capacity: capacity, Security: security}
			if decision.Ready != test.ready {
				t.Fatalf("ready=%v", decision.Ready)
			}
			if _, err := Check(nil, CheckInput{Proposal: test.proposal, Decision: decision}); err != nil {
				t.Fatal(err)
			}
			decision.Ready = !decision.Ready
			if _, err := Check(nil, CheckInput{Proposal: test.proposal, Decision: decision}); err == nil {
				t.Fatal("checker accepted contradictory readiness")
			}
		})
	}
}
