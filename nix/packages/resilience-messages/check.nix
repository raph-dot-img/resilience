# The messaging program against the real libraries (ADR 0018): one
# Reticulum instance and two accounts' programs in the build sandbox (no
# network needed: programs attach through the instance's local socket).
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
    start() { USER=$1 ${messages}/bin/resilience-messages run --dir $PWD/$1 --client-config $PWD/client-config > $1.log 2>&1 & }

    # Refused without doing anything.
    ${messages}/bin/resilience-messages run --dir $PWD/x --client-config $PWD/missing && exit 1
    ${messages}/bin/resilience-messages bogus && exit 1
    test ! -e x

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

    kill %1 %2 %3 2>/dev/null || true
    kill $(jobs -p) 2>/dev/null || true
    mkdir $out
    cat ann/messages/*.json bob/messages/*.json > $out/messages.jsonl
  ''
