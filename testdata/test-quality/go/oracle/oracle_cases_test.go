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

// A bare `go` with neither a sync primitive nor an intent cue is not enough to tag.
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

// Round 9: a typed blank declaration after a setup binding is still a compile-time check
// (cosmos-sdk internal/blockstm/txnrunner_test.go).
func TestCompile_blankDeclAfterSetup(t *testing.T) {
	decoder := mockTxDecoder

	var _ sdk.TxRunner = NewSTMRunner(decoder, []storetypes.StoreKey{}, 1, false, testCoinDenomFunc)
}

// An interface-named test whose only statement is a subtest that discards a constructor result
// (mattermost pluginapi poster).
func TestInterface_constructorDiscarded(t *testing.T) {
	t.Run("Plugin API satisfy the interface", func(t *testing.T) {
		api := &plugintest.API{}
		client := pluginapi.NewClient(api)
		_ = NewPoster(&client.Post, botID)
	})
}

// Discarding a constructor result without an interface cue is not a compile check.
func TestBuild_constructorDiscarded(t *testing.T) {
	cfg := loadConfig()
	_ = NewPoster(&cfg.Post, botID)
}

// Goroutines and a WaitGroup that only prove nothing deadlocks carry no detector intent.
func TestFanOut_noDeadlock(t *testing.T) {
	var wg sync.WaitGroup
	ch := make(chan int)
	for i := 0; i < 8; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			feed.Send(i)
		}()
	}
	go func() { wg.Wait(); close(ch) }()
	for range ch {
	}
}

// The oracle is that Close and Reload return when run in parallel: a deadlock probe.
func TestDeadlockBw_closeAndReload(t *testing.T) {
	wg := sync.WaitGroup{}
	wg.Add(2)
	go func() {
		defer wg.Done()
		for range 100 {
			hs.Close()
			hs.Open()
		}
	}()
	go func() {
		defer wg.Done()
		for range 100 {
			se.Broadcast()
		}
	}()
	wg.Wait()
}

// Producer and consumer goroutines wired by channels: liveness only.
func TestProduceConsume_pipeline(t *testing.T) {
	buffer := NewInfiniteBuffer()
	producer := make(chan Message)
	consumer := make(chan *Message)
	go buffer.StartListening(producer)
	go buffer.Consume(consumer)
	go func() {
		for i := 0; i < 10000; i++ {
			producer <- Message{}
		}
	}()
	for i := 0; i < 10000; i++ {
		<-consumer
	}
}

// Parallel mutation named in a comment counts even without a sync primitive.
// Regression test to ensure we can safely mutate labels in parallel
func TestParallelLabels_commentOnly(t *testing.T) {
	for i := range 100 {
		go func() {
			m.With(fmt.Sprint(i)).Record(1)
		}()
	}
}

// The race detector is the oracle and the concurrency lives in the callee: no `go` here.
// TestProxyDataRace find the concurrent data race bug ( go test -race -run TestProxyDataRace -v ).
func TestProxyDataRace(t *testing.T) {
	p := NewProxy(logger, endpoints)
	_ = p.MetricMetadata(req, s)
}

// A word that merely contains the letters, as in Trace or Grace, is no cue.
func TestTraceExporter_noGoroutines(t *testing.T) {
	exp := NewTraceExporter()
	exp.Export(span)
}

// Subtests delegating to a helper that only skips are a stub even through a table loop (dolt).
func TestStub_tableLoopToSkipHelper(t *testing.T) {
	var tests = []identityTest{
		{
			name: "smoke test",
			left: []table{{name: "t", cols: []column{{name: "pk", enc: val.Int32Enc}}}},
		},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			testIdentity(t, test)
		})
	}
}

func testIdentity(t *testing.T, test identityTest) {
	t.Skip("implement me")
}

// A table loop whose helper really runs something is not a stub.
func TestDelegates_tableLoopToWorkingHelper(t *testing.T) {
	tests := []identityTest{{name: "a"}}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			workingHelper(t)
		})
	}
}
