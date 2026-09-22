// hebnix-tsnet-sidecar embeds a Tailscale node (via tsnet) so Hebnix can join
// a headscale-coordinated tailnet for Workshop multiplayer without requiring
// the user to install the official Tailscale client.
//
// Protocol: on startup this process prints exactly one JSON line to stdout
// announcing the loopback port it is listening on, then all further stdout
// is free-form log text (forwarded into Hebnix's log console). Control is a
// single TCP connection to that port carrying newline-delimited JSON in both
// directions: commands in, responses/events out.
package main

import (
	"bufio"
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"log"
	"net"
	"os"
	"sync"
	"time"

	"tailscale.com/ipn/ipnstate"
	"tailscale.com/tsnet"
)

const protocolVersion = 1

type readyMsg struct {
	Type            string `json:"type"`
	ProtocolVersion int    `json:"protocol_version"`
	Port            int    `json:"port"`
}

// ---- commands (read from the control connection) ----

type command struct {
	Cmd        string `json:"cmd"`
	AuthKey    string `json:"auth_key,omitempty"`
	Hostname   string `json:"hostname,omitempty"`
	ControlURL string `json:"control_url,omitempty"`
}

// ---- outgoing messages (written to the control connection) ----

type peerInfo struct {
	TailnetIP string `json:"tailnet_ip"`
	Hostname  string `json:"hostname"`
	Online    bool   `json:"online"`
}

type outMsg struct {
	Type            string     `json:"type"`
	ProtocolVersion int        `json:"protocol_version,omitempty"`
	Ok              bool       `json:"ok,omitempty"`
	Error           string     `json:"error,omitempty"`
	State           string     `json:"state,omitempty"`
	TailnetIP       string     `json:"tailnet_ip,omitempty"`
	Peers           []peerInfo `json:"peers,omitempty"`
	Kind            string     `json:"kind,omitempty"`
}

type sidecar struct {
	mu        sync.Mutex
	srv       *tsnet.Server
	stateDir  string
	tailnetIP string
	state     string // "stopped" | "starting" | "connecting" | "connected" | "backoff"
	peers     map[string]peerInfo
}

func main() {
	stateDir := flag.String("state-dir", "", "directory tsnet persists its node/machine key state in")
	flag.Parse()

	if *stateDir == "" {
		dir, err := os.UserCacheDir()
		if err != nil {
			dir = os.TempDir()
		}
		*stateDir = dir + string(os.PathSeparator) + "Hebnix" + string(os.PathSeparator) + "tsnet-state"
	}
	if err := os.MkdirAll(*stateDir, 0o700); err != nil {
		log.SetOutput(os.Stderr)
		log.Fatalf("failed to create state dir %q: %v", *stateDir, err)
	}

	// Log to stderr only, so stdout stays reserved for the one-line ready
	// handshake plus anything the caller explicitly wants forwarded.
	log.SetOutput(os.Stderr)

	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		log.Fatalf("failed to bind loopback control socket: %v", err)
	}
	port := listener.Addr().(*net.TCPAddr).Port

	sc := &sidecar{stateDir: *stateDir, state: "stopped", peers: map[string]peerInfo{}}

	ready := readyMsg{Type: "ready", ProtocolVersion: protocolVersion, Port: port}
	readyBytes, _ := json.Marshal(ready)
	fmt.Println(string(readyBytes))
	os.Stdout.Sync()

	for {
		conn, err := listener.Accept()
		if err != nil {
			log.Printf("accept error: %v", err)
			continue
		}
		sc.handleConn(conn)
		// A single control connection is expected for the process's life;
		// if it drops, keep listening in case Hebnix reconnects.
	}
}

func (sc *sidecar) handleConn(conn net.Conn) {
	defer conn.Close()
	enc := json.NewEncoder(conn)
	var encMu sync.Mutex
	send := func(m outMsg) {
		encMu.Lock()
		defer encMu.Unlock()
		if err := enc.Encode(m); err != nil {
			log.Printf("failed to write to control connection: %v", err)
		}
	}

	// Push connectivity events to this connection while it's alive.
	stopEvents := make(chan struct{})
	go sc.pumpEvents(send, stopEvents)
	defer close(stopEvents)

	scanner := bufio.NewScanner(conn)
	scanner.Buffer(make([]byte, 0, 4096), 1<<20)
	for scanner.Scan() {
		var cmd command
		if err := json.Unmarshal(scanner.Bytes(), &cmd); err != nil {
			log.Printf("bad command: %v", err)
			continue
		}
		switch cmd.Cmd {
		case "up":
			sc.handleUp(cmd, send)
		case "status":
			send(sc.statusMsg())
		case "down":
			sc.handleDown(send)
		case "shutdown":
			send(outMsg{Type: "down_result", Ok: true})
			sc.handleDown(func(outMsg) {})
			os.Exit(0)
		default:
			log.Printf("unknown command: %q", cmd.Cmd)
		}
	}
}

func (sc *sidecar) handleUp(cmd command, send func(outMsg)) {
	sc.mu.Lock()
	if sc.srv != nil {
		sc.mu.Unlock()
		send(outMsg{Type: "up_result", Ok: true, TailnetIP: sc.tailnetIP})
		return
	}
	sc.state = "starting"
	srv := &tsnet.Server{
		Hostname:   cmd.Hostname,
		AuthKey:    cmd.AuthKey,
		ControlURL: cmd.ControlURL,
		Dir:        sc.stateDir,
		Ephemeral:  true,
		Logf:       func(string, ...any) {}, // tsnet's own verbose logging is too chatty for the shared stderr log stream
	}
	sc.srv = srv
	sc.mu.Unlock()

	sc.setState("connecting")

	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	status, err := srv.Up(ctx)
	if err != nil {
		sc.mu.Lock()
		sc.srv = nil
		sc.mu.Unlock()
		sc.setState("stopped")
		send(outMsg{Type: "up_result", Ok: false, Error: err.Error()})
		return
	}

	ip := ""
	if len(status.TailscaleIPs) > 0 {
		ip = status.TailscaleIPs[0].String()
	}
	sc.mu.Lock()
	sc.tailnetIP = ip
	sc.mu.Unlock()
	sc.setState("connected")

	send(outMsg{Type: "up_result", Ok: true, TailnetIP: ip})
}

func (sc *sidecar) handleDown(send func(outMsg)) {
	sc.mu.Lock()
	srv := sc.srv
	sc.srv = nil
	sc.tailnetIP = ""
	sc.peers = map[string]peerInfo{}
	sc.mu.Unlock()
	if srv != nil {
		_ = srv.Close()
	}
	sc.setState("stopped")
	send(outMsg{Type: "down_result", Ok: true})
}

func (sc *sidecar) setState(s string) {
	sc.mu.Lock()
	sc.state = s
	sc.mu.Unlock()
}

func (sc *sidecar) statusMsg() outMsg {
	sc.mu.Lock()
	defer sc.mu.Unlock()
	peers := make([]peerInfo, 0, len(sc.peers))
	for _, p := range sc.peers {
		peers = append(peers, p)
	}
	return outMsg{Type: "status_result", State: sc.state, TailnetIP: sc.tailnetIP, Peers: peers}
}

// pumpEvents polls tsnet's local client for peer/state changes and pushes
// them as unsolicited "event" frames. tsnet doesn't expose a push-based
// subscription over the public API used here, so this uses a short poll
// interval instead of a long-lived watcher.
func (sc *sidecar) pumpEvents(send func(outMsg), stop <-chan struct{}) {
	ticker := time.NewTicker(2 * time.Second)
	defer ticker.Stop()
	for {
		select {
		case <-stop:
			return
		case <-ticker.C:
			sc.mu.Lock()
			srv := sc.srv
			sc.mu.Unlock()
			if srv == nil {
				continue
			}
			lc, err := srv.LocalClient()
			if err != nil {
				continue
			}
			status, err := lc.Status(context.Background())
			if err != nil {
				continue
			}
			sc.diffPeers(status, send)
		}
	}
}

func (sc *sidecar) diffPeers(status *ipnstate.Status, send func(outMsg)) {
	sc.mu.Lock()
	defer sc.mu.Unlock()
	seen := map[string]bool{}
	for _, peer := range status.Peer {
		if len(peer.TailscaleIPs) == 0 {
			continue
		}
		ip := peer.TailscaleIPs[0].String()
		seen[ip] = true
		online := peer.Online
		prev, existed := sc.peers[ip]
		if !existed || prev.Online != online {
			info := peerInfo{TailnetIP: ip, Hostname: peer.HostName, Online: online}
			sc.peers[ip] = info
			kind := "peer_offline"
			if online {
				kind = "peer_online"
			}
			go send(outMsg{Type: "event", Kind: kind, TailnetIP: ip})
		}
	}
	for ip := range sc.peers {
		if !seen[ip] {
			delete(sc.peers, ip)
			go send(outMsg{Type: "event", Kind: "peer_offline", TailnetIP: ip})
		}
	}
}
