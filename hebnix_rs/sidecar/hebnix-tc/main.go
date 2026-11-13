// hebnix-tc carries Workshop multiplayer (Rocket League LAN play) over
// tailcat: userspace WireGuard with no TUN device, no special privileges and
// no VPN. Every player runs one, and every player sees everyone else:
//
//   - Each node runs a tailcat server and announces its token (a bearer
//     secret) to the Hebnix directory, which answers with everyone else
//     currently announced (see sidecar/directory). The node then connects
//     to each of them as a client.
//   - Each peer gets a loopback alias on this machine (127.10.0.n). Rocket
//     League joins a peer's match at alias:7777, which is proxied to that
//     peer's own RL on its 127.0.0.1:7777.
//   - LAN discovery: RL's LAN browser broadcasts a HostQuery to port 14001
//     and a hosting RL answers with a broadcast HostResponse. Every node
//     holds 14001 shared (SO_REUSEADDR) next to RL and sends what its own RL
//     broadcasts to every peer, both over its clients and to the peers
//     connected to its server. Queries arriving at a node's server are
//     re-broadcast as they are; responses arriving at a node's client are
//     re-broadcast with "ServerID" rewritten to that peer's alias.
//   - Map sync: alias:14790 is forwarded to the peer's map sync. The peer
//     dials its own 127.0.0.1:14790 from a separate loopback address per
//     remote player (127.77.x.y), so per-player limits and blocks still work.
//
// RL is launched without -multihome, so its online play keeps working, and
// the in-game LAN match password is what keeps a match private.
//
// Hebnix reads JSON events, one per line, from stdout (see event below).
// Logs go to stderr. The process exits when stdin closes, so it never
// outlives Hebnix; killing it drops every connection at once.
package main

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"log"
	"net"
	"net/http"
	"net/netip"
	"os"
	"os/exec"
	"sort"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/tailscale/tailcat"
)

const (
	gamePort    = 7777
	mapSyncPort = 14790
	// tunnel-side UDP port of the control flow carrying discovery packets
	controlPort = 14999
	// magicsock reads this during package init, so it has to be in the
	// environment before the process starts
	relayOnlyKnob = "TS_DEBUG_ALWAYS_USE_DERP"
	// the directory's shared key comes from the environment, not a flag,
	// so it doesn't show up in process listings
	directoryKeyEnv = "HEBNIX_DIRECTORY_KEY"
	announceEvery   = 20 * time.Second
)

var (
	relayOnly  = flag.Bool("relay-only", false, "send all traffic through DERP relays so peers never learn each other's public IP")
	name       = flag.String("name", "", "display name shown to other players")
	directory  = flag.String("directory", "", "Hebnix directory URL (announce endpoint)")
	localGame  = flag.String("local-game", "127.0.0.1:7777", "address of this machine's own RL game socket")
	aliasBase  = flag.String("alias-base", "127.10", "first two octets of the loopback aliases given to peers")
	mapSync    = flag.Bool("map-sync", true, "forward map sync (TCP 14790) both ways")
	discPorts  = flag.String("discovery-ports", "14001", "comma-separated RL discovery ports to relay")
	maxPeers   = flag.Int("max-peers", 32, "connect to at most this many other players")
	derpMapURL = flag.String("derpmap-url", "", "DERP map URL (default: tailcat.dev's)")
	stdinEOF   = flag.Bool("exit-on-stdin-eof", true, "exit when stdin closes")
	verbose    = flag.Bool("v", false, "tailcat debug logs on stderr")
)

// event is one line of the stdout protocol. Kind is one of:
//
//	ready   the server is up and the first announce went through
//	peer    a player appeared (Online, with Alias) or went away
//	status  every 5 s: Peers with their path and ping, counters
//	error   Message; Fatal ones are followed by exit
type event struct {
	Kind    string       `json:"event"`
	ID      string       `json:"id,omitempty"`
	Name    string       `json:"name,omitempty"`
	Alias   string       `json:"alias,omitempty"`
	Online  *bool        `json:"online,omitempty"`
	Peers   []peerStatus `json:"peers,omitempty"`
	Stats   *statsEvent  `json:"stats,omitempty"`
	Message string       `json:"message,omitempty"`
	Fatal   bool         `json:"fatal,omitempty"`
}

type peerStatus struct {
	ID    string `json:"id"`
	Name  string `json:"name"`
	Alias string `json:"alias"`
	// "direct", "relay <region>" or "unreachable"
	Path      string   `json:"path"`
	LatencyMs *float64 `json:"latency_ms,omitempty"`
}

type statsEvent struct {
	GamePackets int64 `json:"game_packets"`
	MaxDatagram int64 `json:"max_datagram"`
	// datagrams over tailcat.MaxUDPPayload, which the tunnel drops
	Oversize  int64 `json:"oversize"`
	Discovery int64 `json:"discovery"`
}

var emitMu sync.Mutex

func emit(e event) {
	line, _ := json.Marshal(e)
	emitMu.Lock()
	defer emitMu.Unlock()
	os.Stdout.Write(append(line, '\n'))
}

func fatal(format string, args ...any) {
	msg := fmt.Sprintf(format, args...)
	log.Print(msg)
	emit(event{Kind: "error", Message: msg, Fatal: true})
	os.Exit(1)
}

type counters struct {
	game, max, oversize, discovery atomic.Int64
}

var stats counters

func (c *counters) datagram(n int) {
	c.game.Add(1)
	if n > tailcat.MaxUDPPayload {
		c.oversize.Add(1)
	}
	for {
		m := c.max.Load()
		if int64(n) <= m || c.max.CompareAndSwap(m, int64(n)) {
			return
		}
	}
}

func (c *counters) snapshot() *statsEvent {
	return &statsEvent{
		GamePackets: c.game.Load(),
		MaxDatagram: c.max.Load(),
		Oversize:    c.oversize.Load(),
		Discovery:   c.discovery.Load(),
	}
}

func main() {
	flag.Usage = func() {
		fmt.Fprintf(os.Stderr, "usage: %s [flags] -directory <url>\n", os.Args[0])
		flag.PrintDefaults()
	}
	flag.Parse()
	log.SetOutput(os.Stderr)
	if *relayOnly && os.Getenv(relayOnlyKnob) == "" {
		reexecWithRelayOnly()
	}
	if *directory == "" {
		flag.Usage()
		os.Exit(2)
	}
	ports, err := parsePorts(*discPorts)
	if err != nil {
		fatal("%v", err)
	}
	base, err := netip.ParseAddr(*aliasBase + ".0.0")
	if err != nil || !base.Is4() || !base.IsLoopback() {
		fatal("-alias-base must be two octets of a loopback address, like 127.10")
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	if *stdinEOF {
		go func() {
			io.Copy(io.Discard, os.Stdin)
			cancel()
		}()
	}
	run(ctx, ports, base.As4())
}

func parsePorts(list string) ([]uint16, error) {
	var ports []uint16
	for _, p := range strings.Split(list, ",") {
		n, err := strconv.Atoi(strings.TrimSpace(p))
		if err != nil || n <= 0 || n > 65535 {
			return nil, fmt.Errorf("bad -discovery-ports entry %q", p)
		}
		ports = append(ports, uint16(n))
	}
	return ports, nil
}

func reexecWithRelayOnly() {
	cmd := exec.Command(os.Args[0], os.Args[1:]...)
	cmd.Env = append(os.Environ(), relayOnlyKnob+"=true")
	cmd.Stdin, cmd.Stdout, cmd.Stderr = os.Stdin, os.Stdout, os.Stderr
	err := cmd.Run()
	var exitErr *exec.ExitError
	if errors.As(err, &exitErr) {
		os.Exit(exitErr.ExitCode())
	}
	if err != nil {
		fatal("relay-only restart: %v", err)
	}
	os.Exit(0)
}

func tailcatLogf() func(string, ...any) {
	if *verbose {
		return log.Printf
	}
	return func(string, ...any) {}
}

func randomID() string {
	b := make([]byte, 8)
	rand.Read(b)
	return hex.EncodeToString(b)
}

// ---------------------------------------------------------------- node

type node struct {
	id        string
	aliasBase [4]byte
	relay     *discoveryRelay

	mu          sync.Mutex
	peers       map[string]*peer
	serverFlows map[tailcat.ConnPacketConn]struct{}
}

func run(ctx context.Context, ports []uint16, aliasBase [4]byte) {
	id := os.Getenv("HEBNIX_WORKSHOP_NODE_ID")
	if _, err := hex.DecodeString(id); len(id) != 16 || err != nil {
		id = randomID()
	}
	id = strings.ToLower(id)
	n := &node{
		id:          id,
		aliasBase:   aliasBase,
		peers:       map[string]*peer{},
		serverFlows: map[tailcat.ConnPacketConn]struct{}{},
	}
	relay, err := newDiscoveryRelay(ports, n.forwardLocal)
	if err != nil {
		fatal("discovery: %v", err)
	}
	n.relay = relay

	srv := &tailcat.Server{
		Logf:       tailcatLogf(),
		DERPMapURL: *derpMapURL,
		OnUDP: func(port uint16) func(tailcat.ConnPacketConn) {
			switch port {
			case gamePort:
				return proxyFlowToLocalGame
			case controlPort:
				return n.serveControl
			}
			return nil
		},
		OnTCP: func(port uint16) func(net.Conn) {
			if port != mapSyncPort || !*mapSync {
				return nil
			}
			return func(c net.Conn) {
				local, err := dialMapSync(c.RemoteAddr())
				if err != nil {
					log.Printf("map sync: local dial: %v", err)
					c.Close()
					return
				}
				tailcat.ProxyConns(c, local)
			}
		},
	}
	if err := srv.Start(); err != nil {
		fatal("start: %v", err)
	}
	defer srv.Close()
	token := string(srv.TailcatAddr())

	// A cancelled request may already have registered us on the server.
	// Withdraw with a fresh, short deadline before the parent kills the helper.
	defer withdraw(*directory, n.id, token)
	announced := false
	announce := func() {
		list, err := announceTo(ctx, *directory, n.id, token, *name)
		if ctx.Err() != nil {
			return
		}
		if err != nil {
			log.Printf("directory: %v", err)
			if !announced {
				emit(event{Kind: "error", Message: fmt.Sprintf("could not reach the Workshop multiplayer directory: %v", err)})
			}
			return
		}
		if !announced {
			announced = true
			emit(event{Kind: "ready"})
		}
		n.sync(list)
	}
	announce()
	tick := time.NewTicker(announceEvery)
	status := time.NewTicker(5 * time.Second)
	defer tick.Stop()
	defer status.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-tick.C:
			announce()
		case <-status.C:
			emit(event{Kind: "status", Peers: n.status(), Stats: stats.snapshot()})
		}
	}
}

type directoryEntry struct {
	ID    string `json:"id"`
	Token string `json:"token"`
	Name  string `json:"name"`
}

// announces this node and returns everyone else currently announced
func announceTo(ctx context.Context, url, id, token, name string) ([]directoryEntry, error) {
	body, _ := json.Marshal(directoryEntry{ID: id, Token: token, Name: name})
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, url, bytes.NewReader(body))
	if err != nil {
		return nil, err
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("X-Hebnix-Key", os.Getenv(directoryKeyEnv))
	client := http.Client{Timeout: 10 * time.Second}
	resp, err := client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		msg, _ := io.ReadAll(io.LimitReader(resp.Body, 256))
		return nil, fmt.Errorf("%s: %s", resp.Status, strings.TrimSpace(string(msg)))
	}
	var reply struct {
		Peers []directoryEntry `json:"peers"`
	}
	if err := json.NewDecoder(io.LimitReader(resp.Body, 1<<20)).Decode(&reply); err != nil {
		return nil, err
	}
	var others []directoryEntry
	for _, e := range reply.Peers {
		if e.ID != id && e.ID != "" && strings.HasPrefix(e.Token, "tc") {
			others = append(others, e)
		}
	}
	return others, nil
}

// Older directory deployments rely on the heartbeat expiry instead.
func withdraw(url, id, token string) {
	ctx, cancel := context.WithTimeout(context.Background(), 1200*time.Millisecond)
	defer cancel()
	body, _ := json.Marshal(directoryEntry{ID: id, Token: token})
	req, err := http.NewRequestWithContext(ctx, http.MethodDelete, url, bytes.NewReader(body))
	if err != nil {
		return
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("X-Hebnix-Key", os.Getenv(directoryKeyEnv))
	resp, err := http.DefaultClient.Do(req)
	if err == nil {
		resp.Body.Close()
	}
}

// connects to newly listed players and drops the ones no longer listed
func (n *node) sync(list []directoryEntry) {
	listed := map[string]directoryEntry{}
	for _, e := range list {
		listed[e.ID] = e
	}
	n.mu.Lock()
	var gone []*peer
	for id, p := range n.peers {
		if e, ok := listed[id]; !ok || e.Token != p.token {
			gone = append(gone, p)
			delete(n.peers, id)
		}
	}
	// stable order so who gets an alias doesn't depend on map iteration
	ids := make([]string, 0, len(listed))
	for id := range listed {
		ids = append(ids, id)
	}
	sort.Strings(ids)
	var added []*peer
	for _, id := range ids {
		if _, ok := n.peers[id]; ok || len(n.peers) >= *maxPeers {
			continue
		}
		alias, ok := n.freeAlias()
		if !ok {
			break
		}
		e := listed[id]
		p := &peer{id: id, name: cleanName(e.Name), token: e.Token, alias: alias}
		n.peers[id] = p
		added = append(added, p)
	}
	n.mu.Unlock()

	for _, p := range gone {
		p.close()
		offline := false
		emit(event{Kind: "peer", ID: p.id, Name: p.name, Alias: p.alias.String(), Online: &offline})
	}
	for _, p := range added {
		if err := p.start(n.relay); err != nil {
			log.Printf("peer %s: %v", p.id, err)
			n.mu.Lock()
			delete(n.peers, p.id)
			n.mu.Unlock()
			continue
		}
		online := true
		emit(event{Kind: "peer", ID: p.id, Name: p.name, Alias: p.alias.String(), Online: &online})
	}
}

// caller holds n.mu
func (n *node) freeAlias() (netip.Addr, bool) {
	used := map[netip.Addr]bool{}
	for _, p := range n.peers {
		used[p.alias] = true
	}
	for i := 1; i < 255; i++ {
		a := netip.AddrFrom4([4]byte{n.aliasBase[0], n.aliasBase[1], 0, byte(i)})
		if !used[a] {
			return a, true
		}
	}
	return netip.Addr{}, false
}

func cleanName(s string) string {
	s = strings.TrimSpace(s)
	r := []rune(s)
	if len(r) > 64 {
		r = r[:64]
	}
	return string(r)
}

// what this machine's RL broadcast goes to every peer: over our clients to
// their servers (a query they re-broadcast) and to every client connected
// to our server (a response they rewrite and re-broadcast)
func (n *node) forwardLocal(f []byte) {
	n.mu.Lock()
	peers := make([]*peer, 0, len(n.peers))
	for _, p := range n.peers {
		peers = append(peers, p)
	}
	for flow := range n.serverFlows {
		if _, err := flow.Write(f); err != nil {
			delete(n.serverFlows, flow)
		}
	}
	n.mu.Unlock()
	for _, p := range peers {
		p.sendControl(f)
	}
}

// a remote player's control flow into our server: their RL's LAN queries
// get re-broadcast here; anything else on it is ignored
func (n *node) serveControl(flow tailcat.ConnPacketConn) {
	n.mu.Lock()
	n.serverFlows[flow] = struct{}{}
	n.mu.Unlock()
	defer func() {
		n.mu.Lock()
		delete(n.serverFlows, flow)
		n.mu.Unlock()
		flow.Close()
	}()
	buf := make([]byte, 65535)
	for {
		// clients send a keepalive every few seconds
		flow.SetReadDeadline(time.Now().Add(30 * time.Second))
		k, err := flow.Read(buf)
		if err != nil {
			return
		}
		if port, packet, ok := parseFrame(buf[:k]); ok && !isLANResponse(packet) {
			n.relay.broadcast(port, append([]byte(nil), packet...))
		}
	}
}

func (n *node) status() []peerStatus {
	n.mu.Lock()
	peers := make([]*peer, 0, len(n.peers))
	for _, p := range n.peers {
		peers = append(peers, p)
	}
	n.mu.Unlock()
	out := make([]peerStatus, len(peers))
	var wg sync.WaitGroup
	for i, p := range peers {
		wg.Add(1)
		go func() {
			defer wg.Done()
			out[i] = p.ping()
		}()
	}
	wg.Wait()
	sort.Slice(out, func(i, j int) bool { return out[i].Alias < out[j].Alias })
	return out
}

// ---------------------------------------------------------------- peer

// another player: a tailcat client to their server, plus this machine's
// loopback proxies for them
type peer struct {
	id, name, token string
	alias           netip.Addr

	ctx    context.Context
	cancel context.CancelFunc
	cl     *tailcat.Client
	game   net.PacketConn
	mapLn  net.Listener

	mu      sync.Mutex
	control tailcat.ConnPacketConn
}

func (p *peer) start(relay *discoveryRelay) error {
	p.ctx, p.cancel = context.WithCancel(context.Background())
	p.cl = tailcat.NewClient(tailcat.Addr(p.token))
	p.cl.Logf = tailcatLogf()
	p.cl.DERPMapURL = *derpMapURL
	game, err := listenReuse("udp4", netip.AddrPortFrom(p.alias, gamePort).String())
	if err != nil {
		p.close()
		return fmt.Errorf("game proxy on %v: %w", p.alias, err)
	}
	p.game = game
	go p.gameProxy()
	if *mapSync {
		ln, err := net.Listen("tcp4", netip.AddrPortFrom(p.alias, mapSyncPort).String())
		if err != nil {
			log.Printf("peer %s: map sync forward: %v", p.id, err)
		} else {
			p.mapLn = ln
			go p.mapForward()
		}
	}
	go p.runControl(relay)
	return nil
}

func (p *peer) close() {
	if p.cancel != nil {
		p.cancel()
	}
	if p.game != nil {
		p.game.Close()
	}
	if p.mapLn != nil {
		p.mapLn.Close()
	}
	p.mu.Lock()
	if p.control != nil {
		p.control.Close()
	}
	p.mu.Unlock()
	if p.cl != nil {
		p.cl.Close()
	}
}

func (p *peer) sendControl(f []byte) {
	p.mu.Lock()
	flow := p.control
	p.mu.Unlock()
	if flow != nil {
		flow.Write(f)
	}
}

// our control flow to the peer's server, redialed until the peer goes away:
// LAN responses the peer's RL broadcasts arrive here
func (p *peer) runControl(relay *discoveryRelay) {
	alias := p.alias.As4()
	for p.ctx.Err() == nil {
		flow, err := p.cl.DialUDPPort(p.ctx, controlPort)
		if err != nil {
			log.Printf("peer %s: control flow: %v", p.id, err)
			select {
			case <-time.After(3 * time.Second):
			case <-p.ctx.Done():
			}
			continue
		}
		p.mu.Lock()
		p.control = flow
		p.mu.Unlock()
		stop := make(chan struct{})
		go func() {
			t := time.NewTicker(5 * time.Second)
			defer t.Stop()
			for {
				flow.Write([]byte{0})
				select {
				case <-t.C:
				case <-stop:
					return
				}
			}
		}()
		buf := make([]byte, 65535)
		for {
			k, err := flow.Read(buf)
			if err != nil {
				break
			}
			if port, packet, ok := parseFrame(buf[:k]); ok && isLANResponse(packet) {
				relay.broadcast(port, rewriteBeacon(append([]byte(nil), packet...), alias))
			}
		}
		close(stop)
		p.mu.Lock()
		p.control = nil
		p.mu.Unlock()
		flow.Close()
		if p.ctx.Err() == nil {
			log.Printf("peer %s: control flow closed, redialing", p.id)
			time.Sleep(time.Second)
		}
	}
}

// local RL sockets talking to alias:7777 -> one tunnel flow each to the
// peer's game port
func (p *peer) gameProxy() {
	var mu sync.Mutex
	flows := map[string]tailcat.ConnPacketConn{}
	buf := make([]byte, 65535)
	for {
		k, from, err := p.game.ReadFrom(buf)
		if err != nil {
			mu.Lock()
			for _, f := range flows {
				f.Close()
			}
			mu.Unlock()
			return
		}
		stats.datagram(k)
		key := from.String()
		mu.Lock()
		flow := flows[key]
		mu.Unlock()
		if flow == nil {
			flow, err = p.cl.DialUDPPort(p.ctx, gamePort)
			if err != nil {
				log.Printf("peer %s: game flow dial: %v", p.id, err)
				continue
			}
			mu.Lock()
			flows[key] = flow
			mu.Unlock()
			go func(from net.Addr, flow tailcat.ConnPacketConn) {
				b := make([]byte, 65535)
				for {
					k, err := flow.Read(b)
					if err != nil {
						break
					}
					stats.datagram(k)
					p.game.WriteTo(b[:k], from)
				}
				mu.Lock()
				delete(flows, from.String())
				mu.Unlock()
			}(from, flow)
		}
		flow.Write(buf[:k])
	}
}

func (p *peer) mapForward() {
	for {
		c, err := p.mapLn.Accept()
		if err != nil {
			return
		}
		go func() {
			remote, err := p.cl.DialTCPPort(p.ctx, mapSyncPort)
			if err != nil {
				log.Printf("peer %s: map sync dial: %v", p.id, err)
				c.Close()
				return
			}
			tailcat.ProxyConns(c, remote)
		}()
	}
}

func (p *peer) ping() peerStatus {
	st := peerStatus{ID: p.id, Name: p.name, Alias: p.alias.String(), Path: "unreachable"}
	ctx, cancel := context.WithTimeout(p.ctx, 4*time.Second)
	defer cancel()
	r, err := p.cl.DiscoPing(ctx)
	if err != nil {
		return st
	}
	ms := r.LatencySeconds * 1000
	st.LatencyMs = &ms
	st.Path = fmt.Sprintf("relay %d", r.DERPRegionID)
	if r.Endpoint != "" {
		st.Path = "direct"
	}
	return st
}

// ---------------------------------------------------------------- server side

var (
	remoteSourcesMu sync.Mutex
	remoteSources   = map[netip.Addr]netip.Addr{}
)

// dials this machine's own map sync from a loopback address unique to the
// remote player's tunnel address
func dialMapSync(remote net.Addr) (net.Conn, error) {
	target := fmt.Sprintf("127.0.0.1:%d", mapSyncPort)
	ap, err := netip.ParseAddrPort(remote.String())
	if err != nil {
		return net.Dial("tcp4", target)
	}
	remoteSourcesMu.Lock()
	source, ok := remoteSources[ap.Addr()]
	if !ok {
		k := len(remoteSources) + 1
		source = netip.AddrFrom4([4]byte{127, 77, byte(k >> 8), byte(k)})
		remoteSources[ap.Addr()] = source
	}
	remoteSourcesMu.Unlock()
	d := net.Dialer{LocalAddr: &net.TCPAddr{IP: source.AsSlice()}}
	c, err := d.Dial("tcp4", target)
	if err != nil {
		// a loopback without the whole 127/8 range still works, just
		// without telling players apart
		log.Printf("map sync: dial from %v: %v", source, err)
		return net.Dial("tcp4", target)
	}
	return c, nil
}

// one local UDP socket per remote RL flow, so this machine's RL sees every
// remote player as a distinct 127.0.0.1:port
func proxyFlowToLocalGame(flow tailcat.ConnPacketConn) {
	local, err := net.Dial("udp4", *localGame)
	if err != nil {
		log.Printf("game flow: local dial: %v", err)
		flow.Close()
		return
	}
	done := make(chan struct{}, 2)
	go func() {
		buf := make([]byte, 65535)
		for {
			k, err := flow.Read(buf)
			if err != nil {
				break
			}
			if _, err := local.Write(buf[:k]); err != nil {
				break
			}
		}
		done <- struct{}{}
	}()
	go func() {
		buf := make([]byte, 65535)
		for {
			local.SetReadDeadline(time.Now().Add(2 * time.Minute))
			k, err := local.Read(buf)
			if err != nil {
				break
			}
			if _, err := flow.Write(buf[:k]); err != nil {
				break
			}
		}
		done <- struct{}{}
	}()
	<-done
	flow.Close()
	local.Close()
}

// ---------------------------------------------------------------- discovery

func frame(port uint16, packet []byte) []byte {
	return append(binary.BigEndian.AppendUint16(nil, port), packet...)
}

func parseFrame(b []byte) (uint16, []byte, bool) {
	if len(b) < 3 {
		return 0, nil, false
	}
	return binary.BigEndian.Uint16(b), b[2:], true
}

// a hosting RL's answer to a LAN search (as opposed to a search query)
func isLANResponse(packet []byte) bool {
	return bytes.Contains(packet, []byte("HostResponse")) || serverIDRe.Match(packet)
}

// discoveryRelay holds RL's discovery ports shared with RL itself. What this
// machine's RL broadcasts there goes to forward (framed with its port); what
// comes back from the tunnel is broadcast locally for RL to pick up.
type discoveryRelay struct {
	out      *net.UDPConn
	outPort  int
	forward  func([]byte)
	mu       sync.Mutex
	locals   map[netip.Addr]bool
	localsAt time.Time
	// frames relayed either way in the last recentTTL
	recent map[string]time.Time
}

// RL broadcasts the same packet out of every interface within a few ms, and
// a packet this relay just injected can come back through another node's
// relay when both run on one LAN or machine
const recentTTL = 500 * time.Millisecond

func newDiscoveryRelay(ports []uint16, forward func([]byte)) (*discoveryRelay, error) {
	out, err := net.ListenUDP("udp4", &net.UDPAddr{})
	if err != nil {
		return nil, fmt.Errorf("send socket: %w", err)
	}
	if err := enableBroadcast(out); err != nil {
		return nil, fmt.Errorf("send socket: SO_BROADCAST: %w", err)
	}
	r := &discoveryRelay{
		out:     out,
		outPort: out.LocalAddr().(*net.UDPAddr).Port,
		forward: forward,
		recent:  map[string]time.Time{},
	}
	for _, port := range ports {
		pc, err := listenReuse("udp4", fmt.Sprintf("0.0.0.0:%d", port))
		if err != nil {
			return nil, fmt.Errorf("port %d: %w", port, err)
		}
		go r.listen(port, pc)
	}
	return r, nil
}

func (r *discoveryRelay) listen(port uint16, pc net.PacketConn) {
	buf := make([]byte, 65535)
	for {
		k, from, err := pc.ReadFrom(buf)
		if err != nil {
			fatal("discovery port %d: %v", port, err)
		}
		src, ok := from.(*net.UDPAddr)
		if !ok || src.Port == r.outPort || !r.isLocal(src.IP) {
			continue // our own re-broadcast, or another machine on the real LAN
		}
		f := frame(port, buf[:k])
		if r.seen(f) {
			continue
		}
		stats.discovery.Add(1)
		r.forward(f)
	}
}

func (r *discoveryRelay) broadcast(port uint16, packet []byte) {
	if r.seen(frame(port, packet)) {
		return
	}
	stats.discovery.Add(1)
	r.out.WriteToUDP(packet, &net.UDPAddr{IP: net.IPv4bcast, Port: int(port)})
}

// reports whether this frame was relayed (either way) within recentTTL,
// and records it
func (r *discoveryRelay) seen(key []byte) bool {
	now := time.Now()
	r.mu.Lock()
	defer r.mu.Unlock()
	for k, at := range r.recent {
		if now.Sub(at) > recentTTL {
			delete(r.recent, k)
		}
	}
	_, hit := r.recent[string(key)]
	r.recent[string(key)] = now
	return hit
}

func (r *discoveryRelay) isLocal(ip net.IP) bool {
	addr, ok := netip.AddrFromSlice(ip)
	if !ok {
		return false
	}
	addr = addr.Unmap()
	if addr.IsLoopback() {
		return true
	}
	r.mu.Lock()
	defer r.mu.Unlock()
	if time.Since(r.localsAt) > 10*time.Second {
		r.locals = map[netip.Addr]bool{}
		if addrs, err := net.InterfaceAddrs(); err == nil {
			for _, a := range addrs {
				if prefix, err := netip.ParsePrefix(a.String()); err == nil {
					r.locals[prefix.Addr().Unmap()] = true
				}
			}
		}
		r.localsAt = time.Now()
	}
	return r.locals[addr]
}

func enableBroadcast(c *net.UDPConn) error {
	raw, err := c.SyscallConn()
	if err != nil {
		return err
	}
	var serr error
	raw.Control(func(fd uintptr) { serr = setBroadcast(fd) })
	return serr
}
