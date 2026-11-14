#!/bin/sh
# Release benchmarks. Requires the existing Rust/LLVM toolchain and POSIX utilities.
set -eu
LC_ALL=C
export LC_ALL

usage() {
    printf '%s\n' 'Usage: scripts/bench.sh [--runs N] [--compare OLD.json]' \
        'N defaults to BENCH_RUNS, or 5. Evaluator samples stop at 30 seconds.'
}
fail() { printf 'bench: %s\n' "$*" >&2; exit 1; }
runs=${BENCH_RUNS:-5}
compare=
while [ "$#" -gt 0 ]; do
    case $1 in
        --runs) [ "$#" -ge 2 ] || fail '--runs needs N'; runs=$2; shift 2 ;;
        --compare) [ "$#" -ge 2 ] || fail '--compare needs a JSON file'; [ -z "$compare" ] || fail 'duplicate --compare'; compare=$2; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) usage >&2; fail "unknown argument: $1" ;;
    esac
done
case $runs in ''|*[!0-9]*) fail 'N must be an integer from 1 to 10000' ;; esac
[ "$runs" -ge 1 ] && [ "$runs" -le 10000 ] || fail 'N must be an integer from 1 to 10000'
runs=$(awk -v n="$runs" 'BEGIN { printf "%d",n }')
[ -z "${ELYA_RECORD+x}${ELYA_REPLAY+x}" ] || fail 'unset ELYA_RECORD and ELYA_REPLAY before benchmarking'
if [ -n "$compare" ]; then
    [ -f "$compare" ] && [ -r "$compare" ] || fail "cannot read $compare"
    compare=$(CDPATH= cd "$(dirname "$compare")" && printf '%s/%s' "$(pwd)" "$(basename "$compare")")
fi
root=$(CDPATH= cd "$(dirname "$0")/.." && pwd)
cd "$root"
umask 077
tmp=${TMPDIR:-/tmp}/elya-bench.$$
mkdir "$tmp" || fail "cannot create temporary directory $tmp"
active_pid=
active_pidfile=
active_watchdog=
active_monitor=
publish_dir=
stop_child() {
    if [ -n "$active_pidfile" ] && [ -r "$active_pidfile" ]; then
        IFS= read -r stop_pid < "$active_pidfile" || stop_pid=
        case $stop_pid in ''|*[!0-9]*) ;; *) kill -KILL "$stop_pid" 2>/dev/null || : ;; esac
    fi
    [ -z "$active_pid" ] || kill -KILL "$active_pid" 2>/dev/null || :
    [ -z "$active_watchdog" ] || kill "$active_watchdog" 2>/dev/null || :
    [ -z "$active_monitor" ] || kill "$active_monitor" 2>/dev/null || :
}
cleanup() { stop_child; rm -rf "$tmp"; [ -z "$publish_dir" ] || rm -rf "$publish_dir"; }
trap cleanup 0
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

# Parse JSON instead of grepping it: comparisons must fail on truncated,
# duplicate-key, malformed or incompatible results. This parser accepts the
# complete JSON grammar; result object keys use the schema's identifier names.
cat > "$tmp/compare.awk" <<'AWK'
function die(message,err) { err="cat >&2"; print "bench: " message | err; close(err); exit 1 }
function ws( c) { while (p <= length(input) && substr(input,p,1) ~ /[ \t\r\n]/) p++ }
function hex(s, n,i,c) {
    n=0
    for (i=1;i<=length(s);i++) {
        c=index("0123456789abcdef",tolower(substr(s,i,1)))-1
        if (c<0) die("invalid JSON unicode escape")
        n=n*16+c
    }
    return n
}
function string( out,c,e,h,n) {
    if (substr(input,p++,1)!="\"") die("expected JSON string")
    out=""
    while (p<=length(input)) {
        c=substr(input,p++,1)
        if (c=="\"") return out
        if (c ~ /[[:cntrl:]]/) die("unescaped JSON control character")
        if (c!="\\") { out=out c; continue }
        e=substr(input,p++,1)
        if (e=="\"" || e=="\\" || e=="/") out=out e
        else if (e=="b") out=out sprintf("%c",8)
        else if (e=="f") out=out sprintf("%c",12)
        else if (e=="n") out=out "\n"
        else if (e=="r") out=out "\r"
        else if (e=="t") out=out "\t"
        else if (e=="u") {
            h=substr(input,p,4)
            if (length(h)!=4 || h ~ /[^0-9a-fA-F]/) die("invalid JSON unicode escape")
            p+=4; n=hex(h)
            out=out (n<128 ? sprintf("%c",n) : "\\u" tolower(h))
        } else die("invalid JSON escape")
    }
    die("unterminated JSON string")
}
function value(path,depth, c,key,child,n,token) {
    if (depth>64) die("JSON nesting exceeds 64 levels")
    ws(); c=substr(input,p,1)
    if (c=="{") {
        typ[doc,path]="object"; p++; ws()
        if (substr(input,p,1)=="}") { p++; return }
        while (1) {
            ws(); key=string()
            if (key !~ /^[A-Za-z_][A-Za-z0-9_]*$/) die("unsupported result object key")
            child=path "." key
            if ((doc SUBSEP child) in typ) die("duplicate JSON key: " key)
            ws(); if (substr(input,p++,1)!=":") die("expected JSON colon")
            value(child,depth+1); ws(); c=substr(input,p++,1)
            if (c=="}") return
            if (c!=",") die("expected JSON object separator")
        }
    }
    if (c=="[") {
        typ[doc,path]="array"; p++; n=0; ws()
        if (substr(input,p,1)=="]") { p++; count[doc,path]=0; return }
        while (1) {
            value(path "[" n "]",depth+1); n++; ws(); c=substr(input,p++,1)
            if (c=="]") { count[doc,path]=n; return }
            if (c!=",") die("expected JSON array separator")
        }
    }
    if (c=="\"") { typ[doc,path]="string"; val[doc,path]=string(); return }
    token=substr(input,p)
    if (match(token,/^-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?/)) {
        typ[doc,path]="number"; val[doc,path]=substr(token,1,RLENGTH); p+=RLENGTH; return
    }
    if (substr(token,1,4)=="true" || substr(token,1,4)=="null") {
        typ[doc,path]=substr(token,1,4); val[doc,path]=substr(token,1,4); p+=4; return
    }
    if (substr(token,1,5)=="false") { typ[doc,path]="false"; val[doc,path]="false"; p+=5; return }
    die("invalid JSON value")
}
function read_json(file,id, line,rc) {
    input=""
    while ((rc=(getline line < file))>0) {
        input=input line "\n"
        if (length(input)>33554432) die("result JSON exceeds 32 MiB")
    }
    close(file)
    if (rc<0) die("cannot read result JSON: " file)
    doc=id; p=1; value("",0); ws()
    if (p<=length(input)) die("trailing data in result JSON")
}
function require_type(id,path,type) { if (typ[id,path]!=type) die("invalid result field " path " (expected " type ")") }
function near(a,b) { return (a-b<0 ? b-a : a-b)<0.00000051 }
function backend(id,path,runs, status,n,i,j,v,low,median,peak,rss,arr) {
    require_type(id,path,"object"); require_type(id,path ".status","string")
    status=val[id,path ".status"]
    if (status=="skipped") {
        if (path !~ /\.evaluator$/) die("native backend cannot be skipped")
        require_type(id,path ".reason","string")
        if (val[id,path ".reason"]!="exceeded 30s limit") die("unknown evaluator skip reason")
        require_type(id,path ".timeout_seconds","number")
        if (val[id,path ".timeout_seconds"]!=30) die("invalid evaluator deadline")
        return
    }
    if (status!="ok") die("unknown backend status")
    require_type(id,path ".samples","array")
    n=count[id,path ".samples"]
    if (n!=runs) die("sample count differs from runs")
    low=-1; peak=-1
    for (i=0;i<n;i++) {
        arr=path ".samples[" i "]"
        require_type(id,arr ".wall_seconds","number")
        v=val[id,arr ".wall_seconds"]+0
        if (v<0) die("negative wall time")
        seconds[i]=v
        if (low<0 || v<low) low=v
        rss=typ[id,arr ".peak_rss_kib"]
        if (rss=="number") {
            v=val[id,arr ".peak_rss_kib"]+0
            if (v<0 || v!=int(v)) die("invalid RSS")
            if (v>peak) peak=v
        } else if (rss!="null") die("invalid sample RSS")
    }
    for (i=1;i<n;i++) for (j=i;j>0 && seconds[j]<seconds[j-1];j--) {
        v=seconds[j]; seconds[j]=seconds[j-1]; seconds[j-1]=v
    }
    median=(n%2 ? seconds[int(n/2)] : (seconds[n/2-1]+seconds[n/2])/2)
    require_type(id,path ".median_seconds","number")
    require_type(id,path ".min_seconds","number")
    if (!near(val[id,path ".median_seconds"],median) || !near(val[id,path ".min_seconds"],low)) die("inconsistent timing summary")
    if (peak<0) require_type(id,path ".peak_rss_kib","null")
    else {
        require_type(id,path ".peak_rss_kib","number")
        if (val[id,path ".peak_rss_kib"]!=peak) die("inconsistent RSS summary")
    }
}
function validate(id, runs,n,i,path,name,sha) {
    require_type(id,"","object")
    require_type(id,".schema_version","number")
    if (val[id,".schema_version"]!=1) die("unsupported result schema")
    require_type(id,".suite_fingerprint","string")
    sha=val[id,".suite_fingerprint"]
    if (length(sha)!=40 || sha ~ /[^0-9a-f]/) die("invalid suite fingerprint")
    require_type(id,".runs","number"); runs=val[id,".runs"]+0
    if (runs<1 || runs>10000 || runs!=int(runs)) die("invalid result run count")
    require_type(id,".benchmarks","array"); n=count[id,".benchmarks"]
    if (n<1) die("result has no benchmarks")
    for (i=0;i<n;i++) {
        path=".benchmarks[" i "]"
        require_type(id,path ".name","string"); name=val[id,path ".name"]
        if (name !~ /^[a-z0-9_-]+$/ || ((id SUBSEP name) in names)) die("invalid or duplicate benchmark name")
        names[id,name]=path
        require_type(id,path ".source_fingerprint","string")
        sha=val[id,path ".source_fingerprint"]
        if (length(sha)!=40 || sha ~ /[^0-9a-f]/) die("invalid source fingerprint")
        require_type(id,path ".checksum","string")
        if (val[id,path ".checksum"] !~ /^-?[0-9]+$/) die("invalid checksum")
        backend(id,path ".native",runs); backend(id,path ".evaluator",runs)
    }
}
function ratio(oldpath,newpath,side, a,b,r) {
    a=oldpath "." side; b=newpath "." side
    if (val["new",b ".status"]=="skipped") return "UNVERIFIED (30s limit)"
    if (val["old",a ".status"]=="skipped") return "n/a (previously skipped)"
    a=val["old",a ".median_seconds"]+0; b=val["new",b ".median_seconds"]+0
    if (a==0) return "n/a (old time is zero)"
    r=b/a
    return sprintf("%.3fx%s",r,r>1.10 ? " **SLOWDOWN >10%**" : "")
}
BEGIN {
    read_json(old,"old"); validate("old")
    if (expected_suite!="" && val["old",".suite_fingerprint"]!=expected_suite) die("benchmark workload differs from old run")
    if (new=="") exit
    read_json(new,"new"); validate("new")
    if (val["old",".suite_fingerprint"]!=val["new",".suite_fingerprint"]) die("benchmark workloads differ")
    if (count["old",".benchmarks"]!=count["new",".benchmarks"]) die("benchmark sets differ")
    print ""
    print "Ratios: new median / old median; above 1.10 is a slowdown."
    print ""
    print "| Benchmark | Native ratio | Evaluator ratio |"
    print "| --- | ---: | ---: |"
    for (i=0;i<count["new",".benchmarks"];i++) {
        path=".benchmarks[" i "]"; name=val["new",path ".name"]
        if (!(("old" SUBSEP name) in names)) die("benchmark missing from old run: " name)
        op=names["old",name]
        if (val["old",op ".source_fingerprint"]!=val["new",path ".source_fingerprint"] || val["old",op ".checksum"]!=val["new",path ".checksum"]) die("changed benchmark: " name)
        printf "| %s | %s | %s |\n",name,ratio(op,path,"native"),ratio(op,path,"evaluator")
    }
}
AWK

# Hash only workload files, including vendored modules and fixed text inputs.
# Standard-library/compiler changes remain legitimate comparison targets.
find benches -type f \( -name '*.elya' -o -name '*.txt' \) -print > "$tmp/unsorted-workloads" || fail 'cannot enumerate workload files'
sort "$tmp/unsorted-workloads" > "$tmp/workloads"
[ -s "$tmp/workloads" ] || fail 'no benchmark workload files'
while IFS= read -r source; do
    printf '%s ' "$source"
    git hash-object "$source"
done < "$tmp/workloads" > "$tmp/manifest"
fingerprint=$(git hash-object "$tmp/manifest")
if [ -n "$compare" ]; then
    awk -v old="$compare" -v expected_suite="$fingerprint" -f "$tmp/compare.awk"
fi

# GNU time reports exact process high-water RSS. /proc is a dependency-free
# fallback: wall time has 10ms resolution; VmHWM is sampled, so its maximum is
# a lower bound if the process allocates and exits between samples.
time_bin=
time_kind=
path_time=$(command -v time 2>/dev/null || :)
path_gtime=$(command -v gtime 2>/dev/null || :)
for candidate in /usr/bin/time /bin/time "$path_time" "$path_gtime"; do
    if [ -x "$candidate" ] && "$candidate" -f '%e %M' -o "$tmp/time-probe" sh -c ':' 2>/dev/null; then
        time_bin=$candidate; time_kind=gnu_time; break
    fi
    if [ -x "$candidate" ] && "$candidate" -l sh -c ':' 2> "$tmp/time-probe"; then
        time_bin=$candidate; time_kind=bsd_time; break
    fi
done
if [ "$time_kind" = gnu_time ]; then
    wall_kind=gnu_time_10ms
    rss_kind=gnu_time_maxrss_kib
elif [ "$time_kind" = bsd_time ]; then
    wall_kind=bsd_time_10ms
    rss_kind=bsd_time_maxrss_kib
elif [ -r /proc/uptime ]; then
    wall_kind=proc_uptime_10ms
    rss_kind=proc_vmhwm_sampled_kib
else
    wall_kind=date_1s
    rss_kind=unavailable
fi
poll=0.01
sleep "$poll" 2>/dev/null || poll=1
clock_now() {
    if [ -r /proc/uptime ]; then IFS=' ' read -r clock_value clock_rest < /proc/uptime; printf '%s\n' "$clock_value"
    else
        # POSIX date has no %s. Convert its UTC year/day/time to seconds.
        date -u '+%Y %j %H %M %S' | awk '{
            days=0
            for (y=1970;y<$1;y++) days+=365+(y%4==0 && (y%100!=0 || y%400==0))
            printf "%.0f\n",((days+$2-1)*24+$3)*3600+$4*60+$5
        }'
    fi
}

run_sample() {
    sample_limit=$1; shift
    rm -f "$tmp/pid" "$tmp/done" "$tmp/timed-out" "$tmp/rss" "$tmp/metric" || fail 'cannot reset sample files'
    sample_started=$(clock_now) || fail 'cannot read wall clock'
    active_pidfile=$tmp/pid
    if [ "$time_kind" = gnu_time ]; then
        "$time_bin" -f '%e %M' -o "$tmp/metric" sh -c 'printf "%s\n" "$$" > "$1"; shift; exec "$@"' sh "$tmp/pid" "$@" > "$tmp/stdout" 2> "$tmp/stderr" &
    elif [ "$time_kind" = bsd_time ]; then
        "$time_bin" -l sh -c 'printf "%s\n" "$$" > "$1"; err=$2; shift 2; exec "$@" 2> "$err"' sh "$tmp/pid" "$tmp/stderr" "$@" > "$tmp/stdout" 2> "$tmp/metric" &
    else
        sh -c 'printf "%s\n" "$$" > "$1"; shift; exec "$@"' sh "$tmp/pid" "$@" > "$tmp/stdout" 2> "$tmp/stderr" &
    fi
    active_pid=$!
    # The launcher writes its PID before exec, so the deadline targets the
    # program rather than an intervening /usr/bin/time process.
    while [ ! -s "$tmp/pid" ] && kill -0 "$active_pid" 2>/dev/null; do :; done
    sample_pid=
    if [ -s "$tmp/pid" ]; then IFS= read -r sample_pid < "$tmp/pid" || fail 'cannot read launched process PID'; fi
    case $sample_pid in ''|*[!0-9]*) wait "$active_pid" || :; active_pid=; fail 'program launcher did not publish a PID' ;; esac
    if [ "$rss_kind" = proc_vmhwm_sampled_kib ]; then
        (
            peak=0; seen=false
            finish_rss() { if [ "$seen" = true ]; then printf '%s\n' "$peak"; else printf 'null\n'; fi > "$tmp/rss"; }
            trap 'finish_rss; exit 0' TERM INT HUP
            while kill -0 "$sample_pid" 2>/dev/null; do
                if [ -r "/proc/$sample_pid/status" ]; then
                    while read -r field amount unit; do
                        case $field in VmHWM:) seen=true; if [ "$amount" -gt "$peak" ]; then peak=$amount; fi; break ;; esac
                    done < "/proc/$sample_pid/status" 2>/dev/null || :
                fi
                sleep "$poll"
            done
            finish_rss
        ) &
        active_monitor=$!
    fi
    if [ "$sample_limit" -gt 0 ]; then
        (
            sleeper=
            trap '[ -z "$sleeper" ] || kill "$sleeper" 2>/dev/null || :; exit 0' TERM INT HUP
            sleep "$sample_limit" & sleeper=$!
            wait "$sleeper"
            if [ ! -f "$tmp/done" ] && kill -0 "$sample_pid" 2>/dev/null; then
                : > "$tmp/timed-out"
                kill -KILL "$sample_pid" 2>/dev/null || :
            fi
        ) &
        active_watchdog=$!
    fi
    sample_exit=0
    wait "$active_pid" || sample_exit=$?
    active_pid=
    sample_ended=$(clock_now) || fail 'cannot read wall clock'
    : > "$tmp/done" || fail 'cannot mark sample complete'
    if [ -n "$active_watchdog" ]; then
        kill "$active_watchdog" 2>/dev/null || :
        wait "$active_watchdog" 2>/dev/null || :
        active_watchdog=
    fi
    if [ -n "$active_monitor" ]; then
        wait "$active_monitor" 2>/dev/null || :
        active_monitor=
    fi
    active_pidfile=
    if [ -f "$tmp/timed-out" ]; then return 124; fi
    if [ "$sample_exit" -ne 0 ]; then
        cat "$tmp/stderr" >&2
        fail "program exited with status $sample_exit: $*"
    fi
    if ! cmp -s "$tmp/expected" "$tmp/stdout"; then
        printf 'bench: incorrect stdout from %s\nExpected:\n' "$*" >&2
        cat "$tmp/expected" >&2
        printf 'Actual:\n' >&2
        cat "$tmp/stdout" >&2
        fail 'checksum or return-value mismatch'
    fi
    if [ "$time_kind" = gnu_time ]; then
        read -r sample_wall sample_rss < "$tmp/metric" || fail 'cannot read GNU time measurement'
    elif [ "$time_kind" = bsd_time ]; then
        sample_scale=1
        [ "$(uname -s)" != Darwin ] || sample_scale=1024
        awk -v scale="$sample_scale" '
            $2=="real" { wall=$1; have_wall=1 }
            /maximum resident set size/ { rss=$1; have_rss=1 }
            END { if (!have_wall || !have_rss) exit 1; printf "%s %.0f\n",wall,int((rss+scale-1)/scale) }
        ' "$tmp/metric" > "$tmp/bsd-metric" || fail 'cannot parse BSD time measurement'
        read -r sample_wall sample_rss < "$tmp/bsd-metric" || fail 'cannot read BSD time measurement'
    else
        sample_wall=$(awk -v start="$sample_started" -v end="$sample_ended" 'BEGIN { printf "%.6f", end-start }') || fail 'cannot calculate wall time'
        sample_rss=null
        if [ -r "$tmp/rss" ]; then IFS= read -r sample_rss < "$tmp/rss" || fail 'cannot read sampled RSS'; fi
    fi
    awk -v wall="$sample_wall" -v rss="$sample_rss" 'BEGIN {
        if (wall !~ /^[0-9]+(\.[0-9]+)?$/ || (rss!="null" && rss !~ /^[0-9]+$/)) exit 1
        printf "%.6f %s\n",wall,rss
    }' >> "$sample_file" || fail 'invalid timing measurement'
}

summarize() {
    sort -n -k1,1 "$1" | awk '
        { t[NR]=$1; if ($2!="null" && (!have || $2>peak)) { peak=$2; have=1 } }
        END { if (!NR) exit 1; median=(NR%2 ? t[(NR+1)/2] : (t[NR/2]+t[NR/2+1])/2)
              printf "%.6f %.6f %s\n",median,t[1],have ? peak : "null" }'
}
json_string() {
    printf '%s' "$1" | awk '
        BEGIN { printf "\""; esc["\""]="\\\""; esc["\\"]="\\\\"
                for (i=1;i<32;i++) esc[sprintf("%c",i)]=sprintf("\\u%04x",i) }
        { if (NR>1) printf "\\n"; for (i=1;i<=length($0);i++) {
              c=substr($0,i,1); printf "%s",c in esc ? esc[c] : c } }
        END { printf "\"" }'
}
json_optional() {
    if [ -n "$1" ]; then json_string "$2"; else printf 'null'; fi
}
write_backend() {
    if [ "$1" = skipped ]; then
        printf '{"status":"skipped","reason":"exceeded 30s limit","timeout_seconds":30}'
        return
    fi
    set -- $(summarize "$1") "$1"
    printf '{"status":"ok","median_seconds":%s,"min_seconds":%s,"peak_rss_kib":%s,"samples":[' "$1" "$2" "$3"
    awk '{ printf "%s{\"wall_seconds\":%s,\"peak_rss_kib\":%s}",(NR>1 ? "," : ""),$1,$2 }' "$4"
    printf ']}'
}

printf 'bench: building release compiler\n' >&2
cargo build -p elya-cli --release --features codegen >&2
build_dir=${CARGO_TARGET_DIR:-target}
case $build_dir in /*) ;; *) build_dir=$root/$build_dir ;; esac
elya=$build_dir/release/elya
[ -x "$elya" ] || elya=$elya.exe
[ -x "$elya" ] || fail "release compiler missing: $elya"
commit=$(git rev-parse HEAD)
short_sha=$(git rev-parse --short HEAD)
dirty=false
[ -z "$(git status --porcelain --untracked-files=all -- . ':!benches/results')" ] || dirty=true
started=$(date -u '+%Y-%m-%dT%H:%M:%SZ')
date_slug=$(date -u '+%Y-%m-%dT%H%M%SZ')
cpu=$(awk -F ': ' '/^model name[ \t]*:/ { print $2; exit }' /proc/cpuinfo 2>/dev/null || :)
clang_version=$(clang --version 2>/dev/null | awk 'NR==1 { print }')
{
    printf '{\n  "schema_version":1,\n  "date":'; json_string "$started"
    printf ',\n  "commit":'; json_string "$commit"
    printf ',\n  "dirty":%s,\n  "runs":%s,\n  "suite_fingerprint":' "$dirty" "$runs"; json_string "$fingerprint"
    printf ',\n  "profile":"release",\n  "evaluator_command":"elya run --value",\n  "wall_clock":'; json_string "$wall_kind"
    printf ',\n  "rss_method":'; json_string "$rss_kind"
    printf ',\n  "rss_poll_seconds":%s,\n  "host":{"system":' "$poll"; json_string "$(uname -s)"
    printf ',"release":'; json_string "$(uname -r)"
    printf ',"machine":'; json_string "$(uname -m)"
    printf ',"cpu":'; json_string "$cpu"
    printf ',"rustc":'; json_string "$(rustc --version)"
    printf ',"cargo":'; json_string "$(cargo --version)"
    printf ',"clang":'; json_string "$clang_version"
    printf '},\n  "environment":{"ELYA_GC_STRESS":'; json_optional "${ELYA_GC_STRESS+x}" "${ELYA_GC_STRESS-}"
    printf ',"ELYA_STACK_MIB":'; json_optional "${ELYA_STACK_MIB+x}" "${ELYA_STACK_MIB-}"
    printf ',"ELY_GC_STATS":'; json_optional "${ELY_GC_STATS+x}" "${ELY_GC_STATS-}"
    printf ',"ELYA_CHECK_LEVELS":'; json_optional "${ELYA_CHECK_LEVELS+x}" "${ELYA_CHECK_LEVELS-}"
    printf ',"RUSTFLAGS":'; json_optional "${RUSTFLAGS+x}" "${RUSTFLAGS-}"
    printf ',"CARGO_ENCODED_RUSTFLAGS":'; json_optional "${CARGO_ENCODED_RUSTFLAGS+x}" "${CARGO_ENCODED_RUSTFLAGS-}"
    printf '},\n  "benchmarks":[\n'
} > "$tmp/results.json"
printf '| Benchmark | Native median (s) | min (s) | peak RSS (KiB) | Evaluator median (s) | min (s) | peak RSS (KiB) | Checksum |\n'
printf '| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n'
bench_count=0
for source in benches/*.elya; do
    [ -f "$source" ] || continue
    bench_name=${source#benches/}; bench_name=${bench_name%.elya}
    case $bench_name in ''|*[!a-z0-9_-]*) fail "unsupported benchmark filename: $source" ;; esac
    checksum=$(awk '/^\/\/ checksum: / { if (NF!=3 || $3 !~ /^-?[0-9]+$/) exit 1; value=$3; n++ }
                    END { if (n!=1) exit 1; print value }' "$source") || fail "expected one '// checksum: INTEGER' in $source"
    printf '%s\n0\n' "$checksum" > "$tmp/expected"
    source_fingerprint=$(git hash-object "$source")
    printf 'bench: building %s\n' "$bench_name" >&2
    if ! "$elya" build "$source" -o "$tmp/$bench_name" > "$tmp/build.stdout" 2> "$tmp/build.stderr"; then
        cat "$tmp/build.stdout" "$tmp/build.stderr" >&2; fail "native build failed: $source"
    fi
    [ ! -s "$tmp/build.stderr" ] || cat "$tmp/build.stderr" >&2
    sample_file=$tmp/native.samples
    : > "$sample_file"
    run=1
    while [ "$run" -le "$runs" ]; do
        printf 'bench: %s native %s/%s\n' "$bench_name" "$run" "$runs" >&2
        run_sample 0 "$tmp/$bench_name"
        run=$((run+1))
    done
    read -r native_median native_min native_rss <<SUMMARY
$(summarize "$sample_file")
SUMMARY
    eval_status=ok
    sample_file=$tmp/evaluator.samples
    : > "$sample_file"
    run=1
    while [ "$run" -le "$runs" ]; do
        printf 'bench: %s evaluator %s/%s (30s limit)\n' "$bench_name" "$run" "$runs" >&2
        if run_sample 30 "$elya" run --value "$source"; then :
        else
            sample_status=$?
            [ "$sample_status" -eq 124 ] || fail "evaluator measurement failed: $source"
            eval_status=skipped
            printf 'bench: %s evaluator SKIPPED: exceeded 30s limit; checksum UNVERIFIED\n' "$bench_name" >&2
            break
        fi
        run=$((run+1))
    done
    if [ "$eval_status" = ok ]; then
        read -r eval_median eval_min eval_rss <<SUMMARY
$(summarize "$sample_file")
SUMMARY
        eval_json=$sample_file
    else
        eval_median='SKIPPED (>30s)'; eval_min=-; eval_rss=-; eval_json=skipped
    fi
    [ "$native_rss" != null ] || native_rss=-
    [ "$eval_rss" != null ] || eval_rss=-
    printf '| %s | %s | %s | %s | %s | %s | %s | %s |\n' "$bench_name" "$native_median" "$native_min" "$native_rss" "$eval_median" "$eval_min" "$eval_rss" "$checksum"
    {
        [ "$bench_count" -eq 0 ] || printf ',\n'
        printf '    {"name":'; json_string "$bench_name"
        printf ',"source_fingerprint":'; json_string "$source_fingerprint"
        printf ',"checksum":'; json_string "$checksum"
        printf ',"native":'; write_backend "$tmp/native.samples"
        printf ',"evaluator":'; write_backend "$eval_json"
        printf '}'
    } >> "$tmp/results.json"
    bench_count=$((bench_count+1))
done
[ "$bench_count" -gt 0 ] || fail 'no benches/*.elya programs'
printf '\n  ]\n}\n' >> "$tmp/results.json"
# Validate the just-written document before publishing it, even without --compare.
awk -v old="$tmp/results.json" -f "$tmp/compare.awk"
mkdir -p benches/results
publish_dir=benches/results/.publish.$$
mkdir "$publish_dir" || fail "cannot create result staging directory $publish_dir"
cat "$tmp/results.json" > "$publish_dir/result.json"
result=benches/results/$date_slug-$short_sha.json
suffix=1
# Hard-link creation is atomic and cannot overwrite an existing measurement.
while ! ln "$publish_dir/result.json" "$result" 2>/dev/null; do
    [ -f "$result" ] || [ -d "$result" ] || [ -L "$result" ] || fail "cannot write results to $result"
    suffix=$((suffix+1)); result=benches/results/$date_slug-$short_sha-$suffix.json
done
rm -rf "$publish_dir"
publish_dir=
printf '\nResults: %s\n' "$result"
case $rss_kind in
    proc_vmhwm_sampled_kib) printf 'Timing: /proc uptime (10ms resolution). RSS: sampled VmHWM (%ss interval; lower bound).\n' "$poll" ;;
    gnu_time_maxrss_kib) printf 'Timing and peak RSS: GNU time.\n' ;;
    bsd_time_maxrss_kib) printf 'Timing and peak RSS: BSD time.\n' ;;
    unavailable) printf 'Timing: date (1s resolution). Peak RSS unavailable.\n' ;;
esac
if [ -n "$compare" ]; then
    awk -v old="$compare" -v new="$result" -f "$tmp/compare.awk"
fi
