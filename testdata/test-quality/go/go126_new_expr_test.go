// Label: true negatives. Go 1.26 `new(expr)` does not parse with tree-sitter-go 0.25: error
// recovery leaves the function declaration without a body and the rest of the file in a top-level
// ERROR node, so the assertions are not seen (containers/podman
// pkg/specgen/generate/kube/kube_test.go, copied verbatim, Apache-2.0). A test whose body did not
// parse is never reported. Reported: a parseable test with no assertions.
package fixtures

import (
	"path/filepath"
	"testing"

	"github.com/stretchr/testify/assert"
)

// Positive control; placed first because the parse error swallows whatever follows it.
func TestParseableNoAssertion(t *testing.T) {
	_ = len("x")
}

func TestSetupSecurityContextSeccompProfile(t *testing.T) {
	profileRoot := t.TempDir()
	defaultPath, err := libpod.DefaultSeccompPath()
	assert.NoError(t, err)

	tests := []struct {
		name                   string
		ctr                    *v1.SecurityContext
		pod                    *v1.PodSecurityContext
		seccompAnnotationPaths *SeccompAnnotationPaths
		ctrName                string
		expected               string
		expectedError          string
	}{
		{
			name: "container profile",
			ctr: &v1.SecurityContext{
				SeccompProfile: seccompProfile(v1.SeccompProfileTypeUnconfined, nil),
			},
			expected: "unconfined",
		},
		{
			name: "pod profile",
			pod: &v1.PodSecurityContext{
				SeccompProfile: seccompProfile(v1.SeccompProfileTypeLocalhost, new("profiles/pod.json")),
			},
			expected: filepath.Join(profileRoot, "profiles/pod.json"),
		},
		{
			name: "container overrides pod",
			ctr: &v1.SecurityContext{
				SeccompProfile: seccompProfile(v1.SeccompProfileTypeLocalhost, new("profiles/container.json")),
			},
			pod: &v1.PodSecurityContext{
				SeccompProfile: seccompProfile(v1.SeccompProfileTypeUnconfined, nil),
			},
			expected: filepath.Join(profileRoot, "profiles/container.json"),
		},
		{
			name: "container securityContext overrides container annotation",
			ctr: &v1.SecurityContext{
				SeccompProfile: seccompProfile(
					v1.SeccompProfileTypeUnconfined,
					nil,
				),
			},
			seccompAnnotationPaths: &SeccompAnnotationPaths{
				containerPaths: map[string]string{
					"test-container": filepath.Join(profileRoot, "annotation.json"),
				},
			},
			ctrName:  "test-container",
			expected: "unconfined",
		},
		{
			name: "pod securityContext overrides pod annotation",
			pod: &v1.PodSecurityContext{
				SeccompProfile: seccompProfile(
					v1.SeccompProfileTypeLocalhost,
					new("profiles/pod.json"),
				),
			},
			seccompAnnotationPaths: &SeccompAnnotationPaths{
				containerPaths: map[string]string{},
				podPath:        filepath.Join(profileRoot, "annotation.json"),
			},
			expected: filepath.Join(profileRoot, "profiles/pod.json"),
		},
		{
			name:     "unset profile uses default",
			expected: defaultPath,
		},
		{
			name: "RuntimeDefault profile",
			ctr: &v1.SecurityContext{
				SeccompProfile: seccompProfile(v1.SeccompProfileTypeRuntimeDefault, nil),
			},
			expected: defaultPath,
		},
		{
			name: "reject empty seccomp profile type",
			ctr: &v1.SecurityContext{
				SeccompProfile: &v1.SeccompProfile{},
			},
			expectedError: "invalid seccomp profile type",
		},
		{
			name: "reject absolute localhost profile",
			ctr: &v1.SecurityContext{
				SeccompProfile: seccompProfile(
					v1.SeccompProfileTypeLocalhost,
					new("/etc/seccomp.json"),
				),
			},
			expectedError: "must be a relative path",
		},
		{
			name: "reject localhost profile with backstep",
			ctr: &v1.SecurityContext{
				SeccompProfile: seccompProfile(
					v1.SeccompProfileTypeLocalhost,
					new("profiles/../seccomp.json"),
				),
			},
			expectedError: "must not contain '..'",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			s := &specgen.SpecGenerator{}
			err := setupSecurityContext(s, tt.ctr, tt.pod, profileRoot, tt.seccompAnnotationPaths, tt.ctrName)
			if tt.expectedError != "" {
				assert.ErrorContains(t, err, tt.expectedError)
				return
			}

			assert.NoError(t, err)
			assert.Equal(t, tt.expected, s.SeccompProfilePath)
		})
	}
}
