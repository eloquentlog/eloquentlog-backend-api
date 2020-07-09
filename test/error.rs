use rocket::http::Status;

use crate::run_test;

#[test]
fn test_404_not_found() {
    run_test(|client, _, _, _| {
        let mut res = client.get("/unknown-path").dispatch();
        assert_eq!(res.status(), Status::NotFound);
        assert!(res
            .body_string()
            .unwrap()
            .contains("'/unknown-path' is not found"));
    });

    run_test(|client, _, _, _| {
        let mut res = client.get("/_/unknown-path").dispatch();
        assert_eq!(res.status(), Status::NotFound);
        assert!(res
            .body_string()
            .unwrap()
            .contains("'/_/unknown-path' is not found"));
    });
}
