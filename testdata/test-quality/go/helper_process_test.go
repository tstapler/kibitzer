// Label: Go subprocess re-exec stubs (cli/cli, os/exec's TestHelperProcess idiom) are entry points
// the parent test re-executes; their assertions live in the parent. Only the `os.Exit` makes a
// body one: a "Helper" name or an env-var guard alone does not.
package fixtures

import (
	"fmt"
	"os"
	"testing"
)

func TestHelperProcess(t *testing.T) {
	if os.Getenv("GO_WANT_HELPER_PROCESS") != "1" {
		return
	}
	fmt.Fprint(os.Stdout, os.Getenv("HELPER_STDOUT"))
	os.Exit(0)
}

func TestExitOnSomePath(t *testing.T) {
	if err := run(os.Args[3:]); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func TestNamedHelperButNoExit(t *testing.T) {
	if os.Getenv("GO_WANT_HELPER_PROCESS") != "1" {
		return
	}
	fmt.Fprint(os.Stdout, "x")
}

func TestEmptyHelperProcess(t *testing.T) {
}
