// Label: true negative. Snapshot/golden oracle (pitfall P10).
package fixtures

import (
	"testing"

	"github.com/bradleyjkemp/cupaloy"
)

func TestSnapshot(t *testing.T) {
	cupaloy.SnapshotT(t, render())
}

func TestSnapshotNoHandle(t *testing.T) {
	render()
	cupaloy.Snapshot()
}
