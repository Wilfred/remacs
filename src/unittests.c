#include <config.h>

#include <stdlib.h>

#include "lisp.h"
#include "thread.h"


extern bool dumping;
extern void do_init_stage ();
extern bool using_utf8 ();

bool dumping;

extern
void test_font_spec ();

void
run_test_font_spec ()
{
  test_font_spec ();
}

void
run_test_vectors ()
{
  test_vectors ();
}

int main (int argc, char **argv)
{
  void *stack_bottom_variable;

  /* Record (approximately) where the stack begins.  */
  stack_bottom = (char *) &stack_bottom_variable;
  dumping = false;
  initialized = false;

# ifdef GNU_LINUX
  {
    char *heap_start = my_heap_start ();
    heap_bss_diff = heap_start - max (my_endbss, my_endbss_static);
  }
# endif

  unexec_init_emacs_zone ();

  emacs_backtrace (-1);

  text_quoting_flag = using_utf8 ();

  inhibit_window_system = 0;

  do_init_stage ();
  init_alloc ();
  init_eval ();
  init_random ();

#if defined HAVE_JSON && !defined WINDOWSNT
  init_json ();
#endif

  initialized = true;

  run_test_vectors ();
  run_test_font_spec ();
}
