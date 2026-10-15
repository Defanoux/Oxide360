#include <Standard_Type.hxx>
#include <bindings_common.hxx>
#include <OSD.hxx>
#include <csignal>

inline rust::String type_name(const Handle_Standard_Type &handle) { return std::string(handle->Name()); }

// opencad patch: turn a crash inside OCCT (a segfault deep in a fillet, say)
// into an exception that the try/OCC_CATCH_SIGNALS wrappers catch. OCCT's
// handlers also take Ctrl+C and friends; give those back their usual meaning.
inline void opencad_catch_signals() {
  OSD::SetSignal(OSD_SignalMode_Set, false);
  for (int s : {SIGINT, SIGHUP, SIGQUIT, SIGTERM}) {
    std::signal(s, SIG_DFL);
  }
}
