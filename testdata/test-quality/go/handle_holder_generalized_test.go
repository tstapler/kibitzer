// Label: true negatives. A pointer or positional literal carrying the testing handle reaches a
// collaborator whose methods fail the test through that field (vitess vstreamer TestSpec),
// whether directly, via a nested helper, or via a type declared in a sibling file. Reported:
// a same-file holder whose methods never touch the field.
package fixtures

import (
	"testing"

	"github.com/stretchr/testify/require"
)

type spec struct {
	t     *testing.T
	name  string
	steps []string
}

func (s *spec) Run() {
	for _, step := range s.steps {
		func() {
			s.verify(step)
		}()
	}
}

func (s *spec) verify(step string) {
	require.NoError(s.t, nil)
}

func TestPointerLiteralMultiField(t *testing.T) {
	ts := &spec{t: t, name: "a", steps: []string{"x"}}
	ts.Run()
}

func TestHolderInSubtestClosure(t *testing.T) {
	t.Run("sub", func(t *testing.T) {
		ts := &spec{t: t, name: "b"}
		ts.Run()
	})
}

type viaHelper struct {
	t *testing.T
}

func (v *viaHelper) Exec() { checkNoErr(v.t, nil) }

func checkNoErr(t *testing.T, err error) {
	t.Helper()
	if err != nil {
		t.Fatal(err)
	}
}

func TestFieldPassedToSameFileHelper(t *testing.T) {
	v := &viaHelper{t: t}
	v.Exec()
}

type positional struct {
	t *testing.T
	n int
}

func (p positional) Play() { p.t.Errorf("bad %d", p.n) }

func TestPositionalLiteral(t *testing.T) {
	p := positional{t, 1}
	p.Play()
}

// siblingSpec is declared in another file of the package; its methods are unseen here.
func TestHolderTypeFromSiblingFile(t *testing.T) {
	ts := &siblingSpec{t: t, rows: 3}
	ts.Replay()
}

type idleSpec struct {
	t    *testing.T
	rows int
}

func (s *idleSpec) Tick() { s.rows++ }

func TestHolderMethodsIgnoreField(t *testing.T) {
	ts := &idleSpec{t: t, rows: 3}
	ts.Tick()
}

type idleViaHelper struct {
	t *testing.T
}

func (s *idleViaHelper) Note() { noteOnly(s.t.Name()) }

func noteOnly(name string) {}

func TestHolderHelperIgnoresHandle(t *testing.T) {
	s := &idleViaHelper{t: t}
	s.Note()
}

type posWriter struct {
	t *testing.T
}

func (w *posWriter) Write(s string) { w.t.Error(s) }

func TestPositionalHolderViaConstructor(t *testing.T) {
	w := posWriter{t}
	newHandler(&w).Handle("x")
}
