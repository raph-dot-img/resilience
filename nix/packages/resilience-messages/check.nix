# The messaging program against the real libraries (ADR 0018, 0019, 0025): one
# Reticulum instance, two accounts' programs and the message store in the
# build sandbox (no network needed: programs attach through the instance's
# local socket).
{
  runCommand,
  jq,
  reticulum,
  messages,
}:

runCommand "resilience-messages-check"
  {
    nativeBuildInputs = [ jq ];
  }
  ''
    set -eu
    export HOME=$PWD/home
    mkdir -p rns ann bob "$HOME"
    key=$(head -c 32 /dev/urandom | od -An -v -tx1 | tr -d ' \n')
    cat > rns/config <<EOF
    [reticulum]
      share_instance = Yes
      instance_name = check
      rpc_key = $key
      enable_transport = No
    [logging]
      loglevel = 3
    [interfaces]
    EOF
    cp rns/config client-config
    ${reticulum}/bin/rnsd --config $PWD/rns > rnsd.log 2>&1 &
    sleep 5

    state() { jq -r .state "$1/messages/$2.json" 2>/dev/null || echo missing; }
    wait_state() {
      for _ in $(seq 60); do
        [ "$(state "$1" "$2")" = "$3" ] && return 0
        sleep 1
      done
      echo "$1/$2 is '$(state "$1" "$2")', not '$3'" >&2
      cat "$1"/messages/*.json rnsd.log ann.log bob.log >&2 || true
      exit 1
    }
    send() { # dir id to text
      jq -n --arg to "$3" --arg text "$4" '{to: $to, text: $text}' > "$1/outbox/$2.json.tmp"
      mv "$1/outbox/$2.json.tmp" "$1/outbox/$2.json"
    }
    send_file() { # dir id to text name source
      cp "$6" "$1/outbox/$2.file"
      jq -n --arg to "$3" --arg text "$4" --arg file "$5" '{to: $to, text: $text, file: $file}' \
        > "$1/outbox/$2.json.tmp"
      mv "$1/outbox/$2.json.tmp" "$1/outbox/$2.json"
    }
    start() { USER=$1 ${messages}/bin/resilience-messages run --dir $PWD/$1 --client-config $PWD/client-config --local-store $PWD/store-address >> $1.log 2>&1 & }

    # Refused without doing anything.
    ${messages}/bin/resilience-messages run --dir $PWD/x --client-config $PWD/missing && exit 1
    ${messages}/bin/resilience-messages bogus && exit 1
    ${messages}/bin/resilience-messages node --client-config $PWD/client-config && exit 1
    ${messages}/bin/resilience-messages node --dir $PWD/x --client-config $PWD/client-config --limit-mb 0 && exit 1
    ${messages}/bin/resilience-messages run --dir $PWD/x --limit-mb 5 && exit 1
    test ! -e x

    # Without the machine's instance, a program waits for it instead of
    # becoming the instance itself.
    sed 's/instance_name = check/instance_name = nothere/' client-config > other-config
    rc=0
    ${messages}/bin/resilience-messages run --dir $PWD/x --client-config $PWD/other-config > x.log 2>&1 || rc=$?
    test $rc = 75
    grep -q "not running yet" x.log
    rm -rf x

    start ann
    ann_pid=$!
    for _ in $(seq 30); do test -s ann/address && break; sleep 1; done
    ann=$(cat ann/address)
    echo "$ann" | grep -Eqx '[0-9a-f]{32}'
    test "$(stat -c %a ann)" = 700
    test "$(stat -c %a ann/identity)" = 600

    # Bob is not there yet: his address is unknown, the message waits.
    start bob
    bob_pid=$!
    for _ in $(seq 30); do test -s bob/address && break; sleep 1; done
    bob=$(cat bob/address)
    kill $bob_pid; wait $bob_pid || true
    test ! -e bob/status.json
    rm -rf ann/reticulum/storage ann/peers.json

    send ann bad zz "hello"
    send ann empty "$bob" "   "
    wait_state ann bad failed
    wait_state ann empty failed
    jq -e '.detail == "the address is not 32 hexadecimal digits" and .text == ""' ann/messages/bad.json

    send ann m1 "$bob" "Water at the school, 9h"
    send ann m2 "$bob" "second, cancelled"
    wait_state ann m1 queued
    sleep 5
    test "$(state ann m1)" = queued
    touch ann/cancel/m2
    wait_state ann m2 failed
    jq -e '.detail == "cancelled"' ann/messages/m2.json

    # The queue survives a restart of the program.
    kill $ann_pid; wait $ann_pid || true
    start ann
    for _ in $(seq 30); do test -e ann/status.json && break; sleep 1; done
    test "$(cat ann/address)" = "$ann"
    test "$(state ann m1)" = queued

    # Bob comes back: the waiting message is delivered, with its proof,
    # and only then called delivered.
    start bob
    wait_state ann m1 delivered
    in=$(ls bob/messages)
    test "$(echo "$in" | wc -l)" = 1
    jq -e --arg ann "$ann" \
      '.state == "received" and .direction == "in" and .peer == $ann and .verified == true
       and .text == "Water at the school, 9h"' "bob/messages/$in"
    jq -e --arg bob "$bob" '.[$bob].name == "bob"' ann/peers.json
    test "$(ls ann/messages | wc -l)" = 4
    test -z "$(find ann/outbox ann/cancel -type f)"

    # A reply on two lines, and the cancelled message never arrived.
    send bob r1 "$ann" "$(printf 'OK, see you there\nwith cans')"
    wait_state bob r1 delivered
    for _ in $(seq 20); do [ "$(ls ann/messages | wc -l)" = 5 ] && break; sleep 1; done
    jq -e -s 'map(select(.direction == "in")) | length == 1 and .[0].verified == true
              and .[0].text == "OK, see you there\nwith cans"' ann/messages/*.json
    test "$(ls bob/messages | wc -l)" = 2

    # Files (ADR 0020): sent directly, kept by Bob under the message id
    # (never under the name that came with it), the sender's copy removed
    # once delivered.
    head -c 300000 /dev/urandom > payload
    send_file ann f1 "$bob" "The list" "../../Liste des besoins.PDF" payload
    wait_state ann f1 delivered
    test ! -e ann/files/f1.data
    jq -e '.file.name == "Liste des besoins.PDF" and .file.size == 300000' ann/messages/f1.json
    for _ in $(seq 20); do grep -l '"The list"' bob/messages/in-*.json > /dev/null 2>&1 && break; sleep 1; done
    got=$(grep -l '"The list"' bob/messages/in-*.json)
    id=$(jq -r .id "$got")
    jq -e --arg d "$id.pdf" '.file.name == "Liste des besoins.PDF" and .file.size == 300000
                             and .file.data == $d and .verified' "$got"
    cmp payload "bob/files/$id.pdf"
    test "$(stat -c %a "bob/files/$id.pdf")" = 600
    test "$(ls bob/files)" = "$id.pdf"
    # Refused: too large (and not kept), a request whose file is missing.
    head -c 900001 /dev/zero > big
    send_file ann f2 "$bob" "" big.bin big
    jq -n --arg to "$bob" '{to: $to, text: "", file: "gone.txt"}' > ann/outbox/f3.json.tmp
    mv ann/outbox/f3.json.tmp ann/outbox/f3.json
    wait_state ann f2 failed
    wait_state ann f3 failed
    jq -e '.detail == "the file is too large (at most 900 KB)"' ann/messages/f2.json
    jq -e '.detail == "the file to send was not found"' ann/messages/f3.json
    test -z "$(find ann/files ann/outbox -type f)"

    # Store-and-forward (ADR 0019): the machine's message store runs; Bob
    # is away when Ann writes; a copy waits in the store; Ann goes too;
    # Bob comes back and collects it, signature checked with the key he
    # kept from before.
    ${messages}/bin/resilience-messages node --dir $PWD/node --client-config $PWD/client-config \
      --address-file $PWD/store-address --limit-mb 4 > node.log 2>&1 &
    node_pid=$!
    for _ in $(seq 30); do test -s store-address && break; sleep 1; done
    grep -Eqx '[0-9a-f]{32}' store-address
    test "$(stat -c %a node/identity)" = 600
    for _ in $(seq 30); do
      [ "$(jq -r .store ann/status.json)" = "$(cat store-address)" ] && break; sleep 1
    done
    [ "$(jq -r .store ann/status.json)" = "$(cat store-address)" ]
    bob_pid=$(cat bob/status.json | jq -r .pid)
    kill "$bob_pid"
    for _ in $(seq 30); do test ! -e bob/status.json && break; sleep 1; done
    send ann m3 "$bob" "Left with the store"
    head -c 400000 /dev/urandom > map
    send_file ann m4 "$bob" "" "map.txt" map
    wait_state ann m3 queued
    for _ in $(seq 240); do jq -e .stored ann/messages/m3.json > /dev/null 2>&1 && break; sleep 1; done
    jq -e '.state == "sent" and .stored == true' ann/messages/m3.json
    # Above LXMF's default store limit (256 KB), within ours (1000 KB).
    for _ in $(seq 60); do jq -e .stored ann/messages/m4.json > /dev/null 2>&1 && break; sleep 1; done
    jq -e '.state == "sent" and .stored == true and (.too_large_for_store | not)' ann/messages/m4.json
    test -n "$(ls node/lxmf/lxmf/messagestore)"
    ann_pid=$(cat ann/status.json | jq -r .pid)
    kill "$ann_pid"
    for _ in $(seq 30); do test ! -e ann/status.json && break; sleep 1; done
    test ! -e ann/status.json
    start bob
    for _ in $(seq 90); do grep -l "Left with the store" bob/messages/in-*.json > /dev/null 2>&1 && break; sleep 1; done
    got=$(grep -l "Left with the store" bob/messages/in-*.json)
    jq -e --arg ann "$ann" '.peer == $ann and .state == "received"' "$got"
    for _ in $(seq 30); do jq -e .verified "$got" > /dev/null && break; sleep 1; done
    jq -e .verified "$got"
    # Collected once: the store hands it over and Bob keeps one copy.
    test "$(grep -l "Left with the store" bob/messages/in-*.json | wc -l)" = 1
    for _ in $(seq 60); do grep -l '"map.txt"' bob/messages/in-*.json > /dev/null 2>&1 && break; sleep 1; done
    got=$(grep -l '"map.txt"' bob/messages/in-*.json)
    id=$(jq -r .id "$got")
    cmp map "bob/files/$id.txt"

    # Community network (ADR 0025): a sharing machine's announce is heard
    # and recorded, checked; hostile announces are not.
    cat > share_driver.py <<'PY'
    import sys
    sys.path.insert(0, sys.argv[1])
    import resilience_messages as m
    mode, rest = sys.argv[2], sys.argv[3:]
    if mode == "unit":
        assert m.local_address("192.168.1.5") == "192.168.1.5"
        assert m.local_address("fe80::0001") == "fe80::1"
        assert m.local_address("fd00::5") == "fd00::5"
        assert m.local_address("169.254.3.4") == "169.254.3.4"
        for bad in ("8.8.8.8", "127.0.0.1", "::1", "2001:db8::1", "x", 5, None, "192.168.1.5\n", "100.64.0.1"):
            assert m.local_address(bad) is None, bad
        good = m.share_app_data("School library", ["packs", "library"], 80, ["192.168.1.5", "fe80::1"])
        got = m.parse_share_app_data(good)
        assert got == {"name": "School library", "services": ["packs", "library"], "port": 80,
                       "addresses": ["192.168.1.5", "fe80::1"]}, got
        import RNS.vendor.umsgpack as mp
        bads = [
            m.share_app_data("bad/name", ["packs"], 80, ["10.0.0.1"]),
            m.share_app_data(" x", ["packs"], 80, ["10.0.0.1"]),
            m.share_app_data("x", ["shell"], 80, ["10.0.0.1"]),
            m.share_app_data("x", ["packs", "packs"], 80, ["10.0.0.1"]),
            m.share_app_data("x", [], 80, ["10.0.0.1"]),
            m.share_app_data("x", ["packs"], 0, ["10.0.0.1"]),
            m.share_app_data("x", ["packs"], True, ["10.0.0.1"]),
            m.share_app_data("x", ["packs"], 80, []),
            m.share_app_data("x", ["packs"], 80, ["8.8.8.8"]),
            m.share_app_data("x", ["packs"], 80, ["10.0.0.%d" % i for i in range(9)]),
            mp.packb([2, "x", ["packs"], 80, ["10.0.0.1"]]),
            mp.packb({"name": "x"}),
            b"\xc1", b"", b"x" * 401, None, "text",
        ]
        for b in bads:
            assert m.parse_share_app_data(b) is None, b
        open("state.toml", "w").write('interface = "eth1"\nname = "School library"\nservices = ["packs"]\nconnection = ""\n')
        assert m.read_share_state("state.toml") == {"name": "School library", "services": ["packs"], "interface": "eth1"}
        for text in ('interface = "a/b"\nname = "x"\nservices = ["packs"]\n',
                     'interface = "eth1"\nname = "x\\n"\nservices = ["packs"]\n',
                     'interface = "eth1"\nname = "x"\nservices = ["shell"]\n', "garbage ["):
            open("bad.toml", "w").write(text)
            assert m.read_share_state("bad.toml") is None, text
        assert m.read_share_state("missing.toml") is None
        assert all(m.local_address(a) for a in m.interface_addresses("lo"))
        # Machines not heard for an hour are dropped.
        import json, time
        json.dump({"old": {"heard": 1}, "new": {"heard": int(time.time())}}, open("shares-test.json", "w"))
        sh = m.Shares("shares-test.json")
        sh.save(time.time())
        assert list(json.load(open("shares-test.json"))) == ["new"]
        print("share unit checks passed")
    else:
        b = m.Beacon(rest[0], rest[1], rest[2])
        if mode == "beacon":
            m.interface_addresses = lambda i: rest[3].split(",")
        else:  # hostile: these exact bytes, whatever they are
            raw = bytes.fromhex(rest[3])
            b.app_data = lambda: raw
        b.run()
    PY
    py() { ${reticulum}/bin/python -B share_driver.py ${messages}/libexec "$@"; }
    py unit

    # No sharing state: the announcer runs and announces nothing.
    ${messages}/bin/resilience-messages share --dir $PWD/beacon --client-config $PWD/client-config \
      --state $PWD/nothing.toml > beacon0.log 2>&1 &
    b0=$!
    ${messages}/bin/resilience-messages share --client-config $PWD/client-config && exit 1
    sleep 8
    kill $b0; wait $b0 || true
    test "$(jq 'length' bob/shares.json 2>/dev/null || echo 0)" = 0

    printf 'interface = "eth1"\nname = "School library"\nservices = ["packs", "library"]\nconnection = ""\n' > share-state.toml
    py beacon $PWD/beacon $PWD/client-config $PWD/share-state.toml "192.168.1.5,fe80::1" > beacon.log 2>&1 &
    # Hostile: a public address, a name with markup, an oversized record.
    pub=$(${reticulum}/bin/python -c 'import RNS.vendor.umsgpack as m; print(m.packb([1, "Free WiFi", ["packs"], 80, ["8.8.8.8"]]).hex())')
    tag=$(${reticulum}/bin/python -c 'import RNS.vendor.umsgpack as m; print(m.packb([1, "<b>x</b>", ["packs"], 80, ["10.0.0.1"]]).hex())')
    py hostile $PWD/h1 $PWD/client-config none "$pub" > h1.log 2>&1 &
    py hostile $PWD/h2 $PWD/client-config none "$tag" > h2.log 2>&1 &
    for _ in $(seq 60); do
      [ "$(jq 'length' bob/shares.json 2>/dev/null || echo 0)" -ge 1 ] && break; sleep 1
    done
    sleep 10
    cat bob/shares.json
    jq -e 'length == 1 and (to_entries[0].value | .name == "School library" and .services == ["packs", "library"]
           and .port == 80 and .addresses == ["192.168.1.5", "fe80::1"] and (.identity | test("^[0-9a-f]{32}$"))
           and (.heard > 0))' bob/shares.json
    test "$(stat -c %a bob/shares.json)" = 600
    test "$(stat -c %a beacon/identity)" = 600
    if grep -q "Free WiFi\|<b>" bob/shares.json; then exit 1; fi

    kill $(jobs -p) 2>/dev/null || true
    mkdir $out
    cat ann/messages/*.json bob/messages/*.json > $out/messages.jsonl
  ''
