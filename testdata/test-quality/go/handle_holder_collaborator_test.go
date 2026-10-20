// Label: true negatives. A same-file collaborator built with the testing handle fails the test
// through that field (coredns plugin/dnstap/handler_test.go). Reported: a collaborator that holds
// the handle but never fails on it.
package fixtures

import "testing"

type failingWriter struct {
	t     *testing.T
	queue []string
}

func (w *failingWriter) Write(s string) {
	if len(w.queue) == 0 {
		w.t.Error("unexpected message")
	}
}

// newHandler lives in another file: the test reaches the writer only through it.
func TestCollaboratorFails(t *testing.T) {
	w := failingWriter{t: t}
	h := newHandler(&w)
	h.Handle("x")
}

type silentWriter struct {
	t     *testing.T
	count int
}

func (w *silentWriter) Record(s string) { w.count++ }

func TestCollaboratorSilent(t *testing.T) {
	w := silentWriter{t: t}
	h := newHandler(&w)
	h.Handle("x")
}
