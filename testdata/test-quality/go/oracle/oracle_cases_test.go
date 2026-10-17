package oracle

// Snippets adapted from real assertion-free tests (stapler-squad, kubernetes) used to unit-test
// the intentional-oracle tagger. Expected tags live in src/test_code/oracle_tag.rs.

func TestCompile_interfaceAssertion(t *testing.T) {
	t.Parallel()
	var _ TerminalDetector = (*fakeTerminalDetector)(nil)
	// If this compiles, the interface is satisfied.
}

func TestCompile_compositeBlankAssign(t *testing.T) {
	_ = &controlplane.Config{
		ControlPlane: controlplaneapiserver.Config{
			EnableLogsSupport: false,
		},
	}
}

func TestRace_waitGroupLoop(t *testing.T) {
	var wg sync.WaitGroup
	for i := 0; i < 8; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			_ = procStateSnapshot(os.Getpid())
		}()
	}
	wg.Wait()
}

func TestRace_namedConcurrent(t *testing.T) {
	done := make(chan struct{})
	go func() {
		c.SetOnUpdated(func(prs []github.UserPR) {})
		done <- struct{}{}
	}()
	c.SetOnUpdated(nil)
	<-done
}

func TestLifecycle_startStop(t *testing.T) {
	t.Parallel()
	c := github.NewUserPRCache()
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	c.Start(ctx)
	c.Start(ctx) // second call is a no-op
	c.Stop()
}

func TestAuditHook_nilLogger(t *testing.T) {
	t.Parallel()
	hook := &LoggingAuditHook{Logger: nil}
	// Should not panic; falls back to slog.Default().
	hook.OnExec(AuditEntry{Command: []string{"echo"}})
}

func TestAddCadvisorFlagsDoesNotPanic(t *testing.T) {
	addCadvisorFlagsDoesNotPanic(pflag.NewFlagSet("test", pflag.ContinueOnError))
}

func TestEndpointSlice_handlers(t *testing.T) {
	p := Proxier{}
	p.endpointsChanges = proxy.NewEndpointsChangeTracker(v1.IPv4Protocol, "", nil, nil)
	p.OnEndpointSliceAdd(&discovery.EndpointSlice{})
	p.OnEndpointSlicesSynced()
}

func TestDiscarded_nameNoDetection(t *testing.T) {
	t.Parallel()
	detector := NewApprovalDetector()
	requests := detector.Detect("This is just regular output")

	// Just verify it doesn't panic
	_ = requests
}

func TestDiscarded_returnsEmpty(t *testing.T) {
	inspector := NewProcessInspector()
	files, err := inspector.OpenFiles(1)
	// Key: must not panic
	_ = err
	_ = files
}

func TestStub_todoOnly(t *testing.T) {
	// TODO(#109831): Enable use of this test and run it.
}

func TestStub_empty(t *testing.T) {
}

func TestUntagged_logOnly(t *testing.T) {
	t.Logf("slice:\n%s", klog.Format(slice))
}

func TestStub_delegatesToSkipHelper(t *testing.T) {
	skipOnlyHelper(t, false, true)
}

func TestStub_delegatesToSkipHelperWithOptions(t *testing.T) {
	skipOnlyHelper(t, true, true, WithAuth("invalid", "invalid"))
}

func TestStub_delegatesToEmptyHelper(t *testing.T) {
	emptyHelper(t)
}

func TestStub_delegatesToTodoHelper(t *testing.T) {
	todoHelper(t)
}

// The helper does real work, so delegation to it is not a stub.
func TestDelegates_toWorkingHelper(t *testing.T) {
	workingHelper(t)
}

// Two levels deep: only one level of delegation is followed.
func TestDelegates_twoLevels(t *testing.T) {
	outerHelper(t)
}

func TestDelegates_skipHelperPlusRealCall(t *testing.T) {
	skipOnlyHelper(t, false, true)
	Parse("x")
}

func skipOnlyHelper(t *testing.T, a, b bool, opts ...Option) {
	// TODO: finish this once the client exposes the method
	t.Skip()
}

func emptyHelper(t *testing.T) {
}

func todoHelper(t *testing.T) {
	// TODO: write this
}

func workingHelper(t *testing.T) {
	Parse("x")
}

func outerHelper(t *testing.T) {
	skipOnlyHelper(t, false, false)
}

// sync.WaitGroup.Go is a goroutine launch even though there is no `go` keyword.
func TestRace_waitGroupGoMethod(t *testing.T) {
	var wg sync.WaitGroup
	for i := 0; i < 100; i++ {
		wg.Go(func() {
			_ = Load(testCfg, true)
		})
	}
	wg.Wait()
}

// A bare `go func` with no WaitGroup/channel: the name carries the race cue.
func TestRace_bareGoNamedConcurrent(t *testing.T) {
	m := NewThreadSafeMap[int, int]()
	go func() {
		for range 10000 {
			m.Set(0, 0)
		}
	}()
	for range 10000 {
		m.Get(0)
	}
}

// A bare `go` with neither a sync primitive nor a race cue is not enough for race-smoke.
func TestBareGo_noCue(t *testing.T) {
	go worker()
	Parse("x")
}

// A construction-style bind (RunServer) followed only by lifecycle calls.
func TestLifecycle_runServerMultiShutdown(t *testing.T) {
	s := RunServer(&opts)
	s.Shutdown()
	s.Shutdown()
}

func TestLifecycle_prefixedVerbs(t *testing.T) {
	s := New(DefaultOptions())
	s.StartProfiler()
	s.CloseAll()
}

// A non-lifecycle call among the lifecycle calls: no-panic, not lifecycle.
func TestStartStop_withSleep(t *testing.T) {
	s := NewServer()
	s.Start()
	time.Sleep(time.Second)
	s.Stop()
}

func TestStartStop_withSubjectCall(t *testing.T) {
	s := NewServer()
	s.Start()
	s.Process("x")
	s.Stop()
}
