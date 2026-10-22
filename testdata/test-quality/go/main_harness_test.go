// Label: true negatives. A flag- or variable-gated call to main() is a coverage/system-test harness
// (beats TestSystem, tidb TestRunMain), and an early-return gate on an env-derived package var in
// a "helper" test is a re-exec'd helper-process mode (beats gotest_test.go). Reported: an
// ungated main() (mattermost TestRunMain), a gated call that is not main(), and gates that are
// neither.
package fixtures

import (
	"flag"
	"os"
	"testing"
)

var systemTest = flag.Bool("systemTest", false, "run system tests")

var isCoverageServer string

var gotestHelperMode = os.Getenv("GOTEST_HELPER") == "1"

var slowMode = os.Getenv("RUN_SLOW") == "1"

var ready = false

func TestSystem(*testing.T) {
	if *systemTest {
		main()
	}
}

func TestRunMain_Gated(t *testing.T) {
	if isCoverageServer == "1" {
		main()
	}
}

func TestRunMain_Ungated(t *testing.T) {
	main()
}

func TestSystem_GatedOrdinaryCall(t *testing.T) {
	if *systemTest {
		doExpensiveThing()
	}
}

func TestSystem_GatedOnError(t *testing.T) {
	if err := setup(); err != nil {
		main()
	}
}

func TestHelper_OK(t *testing.T) {
	if !gotestHelperMode {
		return
	}

	// A trailing comment after the gate must not hide it (beats gotest_test.go:147).
}

func TestHelper_WithPanic(t *testing.T) {
	if !gotestHelperMode {
		return
	}
	panic("Kaputt.")
}

func TestSlow_EnvGateOnly(t *testing.T) {
	if !slowMode {
		return
	}
	parse()
}

func TestHelper_GateNotFromEnv(t *testing.T) {
	if !ready {
		return
	}
	parse()
}
