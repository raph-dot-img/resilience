# The messaging program against the real libraries (ADR 0018, 0019): one
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

    # A reply, and the cancelled message never arrived.
    send bob r1 "$ann" "OK, see you there"
    wait_state bob r1 delivered
    for _ in $(seq 20); do [ "$(ls ann/messages | wc -l)" = 5 ] && break; sleep 1; done
    jq -e -s 'map(select(.direction == "in")) | length == 1 and .[0].verified == true
              and .[0].text == "OK, see you there"' ann/messages/*.json
    test "$(ls bob/messages | wc -l)" = 2

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
    wait_state ann m3 queued
    for _ in $(seq 240); do jq -e .stored ann/messages/m3.json > /dev/null 2>&1 && break; sleep 1; done
    jq -e '.state == "sent" and .stored == true' ann/messages/m3.json
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

    kill $(jobs -p) 2>/dev/null || true
    mkdir $out
    cat ann/messages/*.json bob/messages/*.json > $out/messages.jsonl
  ''
