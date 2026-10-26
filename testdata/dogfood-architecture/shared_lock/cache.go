// Package shared_lock is the negative fixture: two unrelated-looking groups share one
// package-level mutex, so splitting would expose the lock.
package shared_lock

import "sync"

var mu sync.Mutex

type Cache struct{ items map[string]string }

type Entry struct{ key string }

type Counter struct{ n int }

type Gauge struct{ v int }

func (c *Cache) Get(e *Entry) string { mu.Lock(); defer mu.Unlock(); return c.items[e.key] }

func (e *Entry) Key() string { return e.key }

func (c *Counter) Inc(g *Gauge) { mu.Lock(); c.n++; g.v++; mu.Unlock() }

func (g *Gauge) Value() int { return g.v }
