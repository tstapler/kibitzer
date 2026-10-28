// Label: env-gated subprocess entrypoints (vault plugin mains) are not tests: the parent asserts.
// Reported: a gated test that does nothing after the gate, whose remainder is not a serve/run
// call, or a serve call with no environment gate.
package fixtures

import (
	"os"
	"testing"
	"time"
)

func TestPluginMain_Early(t *testing.T) {
	if os.Getenv("PLUGIN_VERSION") == "" {
		return
	}

	v5.ServeMultiplex(postgresql.New)
}

func TestPluginMain_BlankHandle(_ *testing.T) {
	if os.Getenv("PLUGIN_VERSION") == "" && os.Getenv("PLUGIN_METADATA") != "true" {
		return
	}

	plugin := &mockPlugin{}
	flags := meta.FlagSet()
	flags.Parse(nil)
	dbplugin.Serve(plugin)
}

func TestPluginMain_Wrapped(t *testing.T) {
	if _, ok := os.LookupEnv("PLUGIN_VERSION"); ok {
		RunV5()
	}
}

func TestGatedButEmpty(t *testing.T) {
	if os.Getenv("PLUGIN_VERSION") == "" {
		return
	}
}

func TestGatedButNotServing(t *testing.T) {
	if os.Getenv("RUN_SLOW") == "" {
		return
	}
	doExpensiveThing()
}

func TestServeWithoutGate(t *testing.T) {
	v5.Serve(&plugin{})
}

// Subprocess re-exec helper (ollama llm/runner_job_windows_test.go): the parent test asserts.
func TestReexecHelperSleeps(t *testing.T) {
	if os.Getenv("RUNNER_JOB_HELPER") == "" {
		return
	}

	time.Sleep(time.Hour)
}

func TestPluginMain_ReturnHasValueCheck(t *testing.T) {
	if os.Getenv("PLUGIN_VERSION") == "" {
		return
	}
	got := Compute()
	if got != 1 {
		t.Fatalf("got %d", got)
	}
}
